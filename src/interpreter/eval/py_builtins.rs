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
            // すべての値は `object` の派生（タスク 6-10・`int` / `None` / 関数なども）。
            Value::Class(c) if Self::is_object_class(c) => Ok(true),
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
            // すべてのクラス・組み込みの型は `object` の派生（タスク 6-10）。
            Value::Class(base) if Self::is_object_class(base) => Ok(true),
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
        let d = std::rc::Rc::new(std::cell::RefCell::new(crate::interpreter::DictData::new(
            "Any".to_string(),
            "Any".to_string(),
        )));
        self.dict_update_from(&d, evaled, "dict")?;
        Ok(Value::Dict(d))
    }

    /// `dict(..)` / `d.update(..)` の引数で辞書を埋める（タスク 3-1 / 3-6）。位置引数は 0 か 1 個（辞書か、2 要素の組の列）、
    /// キーワード引数は文字列のキーで後から入れる（CPython と同じ順・同じ文言）。入れるのは型を確かめる
    /// `dict_store_checked`（型つきの辞書の `update` も `d[k] = v` と同じ規則）。
    pub(crate) fn dict_update_from(
        &mut self,
        d: &std::rc::Rc<std::cell::RefCell<crate::interpreter::DictData>>,
        evaled: Vec<(Option<String>, Value, bool)>,
        fname: &str,
    ) -> Result<(), String> {
        let (named, positional): (Vec<_>, Vec<_>) = evaled.into_iter().partition(|(k, _, _)| k.is_some());
        if positional.len() > 1 {
            return Err(format!("TypeError: {fname} expected at most 1 argument, got {}", positional.len()));
        }
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
                self.dict_store_checked(d, k, v)?;
            }
        }
        for (k, v, _) in named {
            self.dict_store_checked(d, Value::str(k.expect("partitioned by is_some").as_str()), v)?;
        }
        Ok(())
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

    /// `enumerate(it, start=0)`。**遅延**のジェネレータ（python_builtins_plan.md のタスク 3-5）。
    ///
    /// 取り出すたびに元を 1 つ進めるので、ジェネレータと同じに振る舞う: 作った時点では何も取り出さない・
    /// 途中で抜ければ残りは取り出さない・元と位置を共有する（`enumerate(g)` の後で `next(g)` は続き）・無限の列も使える。
    /// ⚠ 以前は元を**先に全部取り出して**から組の列を作っていた（無限の列で終わらず、元は空になっていた）。
    pub(crate) fn py_enumerate(&mut self, iterable: Value, start: i64) -> Result<Value, String> {
        let src = self.py_eager_iter(iterable)?;
        self.py_call_lazy("__py_enumerate", vec![src, Value::Int(start)], "enumerate")
    }

    /// `zip(*iterables)`。**遅延**のジェネレータ（タスク 3-5）。取り出すたびに各列を前から順に 1 つ進め、どれかが
    /// 尽きたらそこで終わる（それより後ろの列は進めない・CPython と同じ）。
    /// ⚠ 以前は全部の列を**先に最後まで取り出して**いた（長い方・無限の列も回し切った）。
    pub(crate) fn py_zip(&mut self, iterables: Vec<Value>) -> Result<Value, String> {
        let its = iterables.into_iter().map(|v| self.py_eager_iter(v)).collect::<Result<Vec<_>, _>>()?;
        // 2 つの列（いちばん多い形）は専用の本体（行のリストと `tuple` を作らないぶん倍ほど速い・同じ意味）。
        if let [a, b] = its.as_slice() {
            return self.py_call_lazy("__py_zip2", vec![a.clone(), b.clone()], "zip");
        }
        self.py_call_lazy(
            "__py_zip",
            vec![Value::List(std::rc::Rc::new(std::cell::RefCell::new(its)))],
            "zip",
        )
    }

    /// `next(it)` / `next(it, default)`（タスク 3-5）。尽きていれば `default` を返す（無ければ `EndOfIteration`・
    /// `StopIteration` の子なので `except StopIteration` で捕まる）。インスタンスは `__next__` を呼び、それが
    /// `StopIteration`（その子を含む）を出したら `default`。
    pub(crate) fn py_next(&mut self, it: Value, default: Option<Value>) -> Result<Value, String> {
        match it {
            Value::Generator(state) => match self.gen_next(&state)? {
                Some(v) => Ok(v),
                None => default.ok_or_else(|| "EndOfIteration: generator is exhausted".to_string()),
            },
            v @ Value::Instance(_) => {
                let r = self.eval_method_call_evaled(v, "__next__", vec![]);
                match (r, default) {
                    (Err(e), Some(d)) if self.is_stop_iteration(&e) => Ok(d),
                    (r, _) => r,
                }
            }
            other => Err(format!("TypeError: '{}' object is not an iterator", self.type_name(&other))),
        }
    }

    /// 誤りが `StopIteration`（その子の `EndOfIteration` を含む）か。送出した例外（`raise StopIteration`）なら
    /// それを取り除く（`next(it, default)` が受け止めるので）。
    fn is_stop_iteration(&mut self, e: &str) -> bool {
        if e == crate::interpreter::RAISE_SENTINEL {
            let is_stop = matches!(
                self.current_exception.as_ref().map(|r| &r.exception),
                Some(Value::Instance(i)) if i.borrow().class.is_a("StopIteration")
            );
            if is_stop {
                self.current_exception = None;
            }
            return is_stop;
        }
        e.starts_with("EndOfIteration") || e.starts_with("StopIteration")
    }

    /// `map(f, *iterables)`。**遅延**（取り出すたびに `f` を呼ぶ・CPython と同じ）。
    /// 回す値が 2 つ以上のときは `zip`（遅延・タスク 3-5）で組にしてから呼ぶ。
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
            let zipped = self.py_zip(rest)?;
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

    /// `for line in f`（ファイルの行・python_builtins_plan.md のタスク 5-4）。取り出すたびに `f.readline()` を呼ぶ
    /// **遅延**のイテレータ（CPython と同じく位置が 1 行ずつ進むので、途中で抜けて `readline()` を続けられる）。
    /// 閉じたファイルは回す前に誤り（Python の形は CPython の文言）。
    pub(crate) fn py_file_lines(&mut self, file: Value) -> Result<Value, String> {
        if let Value::FileObject(fd) = &file {
            let fd = fd.borrow();
            if fd.is_closed {
                return Err(if fd.py_mode.is_some() {
                    "ValueError: I/O operation on closed file.".to_string()
                } else {
                    "IOError: I/O operation on closed file".to_string()
                });
            }
        }
        self.py_call_lazy("__py_file_lines", vec![file], "file")
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
    /// `print(*objects, sep=' ', end='\n', file=None, flush=False)`（CPython と同じ・python_builtins_plan.md のタスク 5-1）。
    ///
    /// ⚠ 以前はキーワード引数を受ける口が無く、`print(x, end="")` を含む関数・最上位の文が
    ///   `VmForceError` でバイトコードにできなかった（ツリーウォークの入口は名前を捨てて `x ` と出していた）。
    /// ⚠ `file=` は `None` だけ（標準出力の外へ書く手段が無い）。`flush=True` は標準出力を流す。
    pub(crate) fn py_print(&mut self, args: Vec<(Option<String>, Value)>) -> Result<Value, String> {
        let (pos, mut kw) = split_named(args);
        let sep = self.print_str_kw(take_kw(&mut kw, "sep"), "sep", " ")?;
        let end = self.print_str_kw(take_kw(&mut kw, "end"), "end", "\n")?;
        match take_kw(&mut kw, "file") {
            None | Some(Value::None) => {}
            Some(_) => {
                return Err("TypeError: print() file= is not supported (only the standard output)".to_string())
            }
        }
        let flush = take_kw(&mut kw, "flush").is_some_and(|v| self.is_truthy(&v));
        reject_extra_kw("print", &kw)?;
        let parts = pos.iter().map(|v| self.display_str(v)).collect::<Result<Vec<_>, _>>()?;
        self.emit_text(&format!("{}{end}", parts.join(&sep)), flush);
        Ok(Value::None)
    }

    /// `sep=` / `end=` は `None`（既定）か文字列（CPython と同じ文言）。
    fn print_str_kw(&self, v: Option<Value>, name: &str, default: &str) -> Result<String, String> {
        match v {
            None | Some(Value::None) => Ok(default.to_string()),
            Some(Value::Str(s)) => Ok(s.to_string()),
            Some(other) => Err(format!(
                "TypeError: {name} must be None or a string, not {}",
                self.type_name(&other)
            )),
        }
    }

    /// 組み込みのクラス `object` か（タスク 6-10）。
    fn is_object_class(c: &ClassValue) -> bool {
        use crate::interpreter::BuiltinClass as B;
        match c.builtin {
            B::Object => true,
            B::Ordinary | B::Ellipsis | B::NotImplemented => false,
        }
    }

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

gen __py_enumerate(it, start) -> Any:
    mut i = start
    for x in it:
        yield (i, x)
        i = i + 1

gen __py_zip2(a, b) -> Any:
    for x in a:
        mut y = None
        try:
            y = next(b)
        except StopIteration:
            break
        yield (x, y)

gen __py_zip(its) -> Any:
    mut going = len(its) > 0
    while going:
        mut row = []
        for it in its:
            if going:
                try:
                    row.append(next(it))
                except StopIteration:
                    going = False
        if going:
            yield tuple(row)

gen __py_file_lines(f) -> Any:
    mut line = f.readline()
    while len(line) > 0:
        yield line
        line = f.readline()
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
                        qualname: None,
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

impl Interpreter {
    /// `format(value, spec)` の本体（python_builtins_plan.md のタスク 3-4）。`str.format` の `{:spec}` と f-string の
    /// `{x:spec}` も同じここを通る（書式の解釈は `ops::format_spec` の 1 つ）。
    ///
    /// - インスタンスは `__format__(spec)` を呼ぶ（str 以外を返すと CPython と同じ `TypeError`）
    /// - `int` / `bool` / `float` / `str` は書式指定の小言語（`bool` は空の指定なら `True`、それ以外は整数として書く）
    /// - それ以外（`__format__` の無いインスタンス・`None`・list …）は、空の指定なら `str(x)`、そうでなければ
    ///   CPython の `object.__format__` と同じ `TypeError`
    /// - ⚠ `complex` の書式指定は未対応（明示の誤り）
    pub(crate) fn format_value(&mut self, v: &Value, spec: &str) -> Result<String, String> {
        use crate::interpreter::ops::format_spec as fs;
        let type_name = match v {
            Value::Instance(i) => i.borrow().class.name.clone(),
            other => self.type_name(other).to_string(),
        };
        match v {
            Value::Instance(i) if i.borrow().class.methods.contains_key("__format__") => {
                match self.eval_method_call_evaled(v.clone(), "__format__", vec![(None, Value::str(spec), false)])? {
                    Value::Str(s) => Ok(s.to_string()),
                    other => Err(format!("TypeError: __format__ must return a str, not {}", self.type_name(&other))),
                }
            }
            Value::Bool(b) if spec.is_empty() => Ok(if *b { "True" } else { "False" }.to_string()),
            Value::Bool(b) => fs::format_int(i128::from(*b), &fs::parse(spec, &type_name, Some('d'), '>')?, &type_name),
            Value::Int(n) => fs::format_int(i128::from(*n), &fs::parse(spec, &type_name, Some('d'), '>')?, &type_name),
            Value::UInt(n) => fs::format_int(i128::from(*n), &fs::parse(spec, &type_name, Some('d'), '>')?, &type_name),
            Value::Float(f) => fs::format_float(*f, &fs::parse(spec, &type_name, None, '>')?, &type_name),
            Value::Str(s) => fs::format_str(s, &fs::parse(spec, &type_name, Some('s'), '<')?, &type_name),
            _ if spec.is_empty() => self.display_str(v),
            Value::Complex(..) => Err("ValueError: format specifiers for complex numbers are not supported".to_string()),
            _ => Err(format!("TypeError: unsupported format string passed to {type_name}.__format__")),
        }
    }

    /// 組み込みの `format(value, format_spec='')`（タスク 3-4）。文言は CPython と同じ。
    pub(crate) fn py_format(&mut self, args: Vec<Value>) -> Result<Value, String> {
        let (v, spec) = match args.as_slice() {
            [v] => (v, ""),
            [v, Value::Str(s)] => (v, &**s),
            [_, other] => {
                return Err(format!("TypeError: format() argument 2 must be str, not {}", self.type_name(other)))
            }
            [] => return Err("TypeError: format expected at least 1 argument, got 0".to_string()),
            _ => return Err(format!("TypeError: format expected at most 2 arguments, got {}", args.len())),
        };
        let spec = spec.to_string();
        let v = v.clone();
        self.format_value(&v, &spec).map(Value::str)
    }
}

/// 組み込みのクラス `property`（python_builtins_plan.md のタスク 6-1）。CPython 3.12 の `property` と同じ振る舞いを
/// Arrow のクラスで書いたもの。読み書きはディスクリプタのプロトコル（タスク 1-12・`eval/attrs.rs`）が
/// `__get__` / `__set__` へ回す。
/// - 文言は CPython と同じ（`property 'x' of 'C' object has no setter`。名前が無ければ `property of 'C' object ..`）。
///   名前は `__set_name__` で入る（変換器がクラスの定義の後で呼ぶ。CPython ではクラスを作るときに `type.__new__` が呼ぶ）。
/// - `getter` / `setter` / `deleter` は名前ごと写した新しい property を返す（CPython 3.12 の `property_copy`）。
/// - ⚠ CPython 3.12 の property は `__name__` を持たない（3.13 から）ので、名前は `private` のフィールドに置く。
/// - ⚠ `__doc__` は渡した `doc` だけ（CPython は `doc` が無ければ getter の `__doc__` を引き継ぐ。変換器が getter の
///   docstring を `doc` に渡すので、変換したクラスでは同じになる・タスク 6-7）。
/// - ⚠ `obj` を書き換える getter / setter に写しを渡さないよう、仮引数は `mut obj`。
/// - ⚠ `__get__` は `mut self`: クラスから読む `C.x` は property **そのもの**を返す（`C.x is C.x`・`C.x.__set_name__(..)`
///   が置いてある property に届く）。`self` のままだと `return self` が写しを返す（Arrow の値の規則）。
const PY_PROPERTY_SRC: &str = r#"class property:
    mut fget: Any
    mut fset: Any
    mut fdel: Any
    mut __doc__: Any
    private:
    mut _name: Any
    public:

    fn __init__(mut self, fget: Any = None, fset: Any = None, fdel: Any = None, doc: Any = None) -> None:
        self.fget = fget
        self.fset = fset
        self.fdel = fdel
        self.__doc__ = doc
        self._name = None

    fn __set_name__(mut self, owner: Any, name: Any) -> None:
        self._name = name

    fn __get__(mut self, mut obj: Any, owner: Any = None) -> Any:
        if obj is None:
            return self
        if self.fget is None:
            raise AttributeError(self._missing(obj, "getter"))
        return self.fget(obj)

    fn __set__(self, mut obj: Any, value: Any) -> None:
        if self.fset is None:
            raise AttributeError(self._missing(obj, "setter"))
        self.fset(obj, value)

    fn __delete__(self, mut obj: Any) -> None:
        if self.fdel is None:
            raise AttributeError(self._missing(obj, "deleter"))
        self.fdel(obj)

    fn getter(self, fget: Any) -> property:
        return self._copy(fget, self.fset, self.fdel)

    fn setter(self, fset: Any) -> property:
        return self._copy(self.fget, fset, self.fdel)

    fn deleter(self, fdel: Any) -> property:
        return self._copy(self.fget, self.fset, fdel)

    private:
    fn _copy(self, fget: Any, fset: Any, fdel: Any) -> property:
        mut p = property(fget, fset, fdel, self.__doc__)
        p._name = self._name
        return p

    fn _missing(self, obj: Any, what: str) -> str:
        if self._name is None:
            return f"property of '{type(obj).__name__}' object has no {what}"
        return f"property '{self._name}' of '{type(obj).__name__}' object has no {what}"
"#;

/// 組み込みのクラス `cached_property`（`functools.cached_property`・python_builtins_plan.md のタスク 6-5）。
/// CPython 3.12 と同じ振る舞いを Arrow のクラスで書いたもの。
/// - 非データディスクリプタ（`__get__` だけ）。初めて読んだときに `func(obj)` を計算してインスタンスの
///   **同じ名前のフィールド**へ入れる。以後はフィールドが勝つ（`get_attr_val` はフィールドを非データディスクリプタより先に見る）。
///   CPython がインスタンスの `__dict__` へ入れるのと同じ効き方。フィールドは変換器が宣言する（Arrow のインスタンスに
///   `__dict__` は無い）。
/// - `obj.x = v` も同じフィールドへ入る（`attr_assign_evaled` が非データディスクリプタと同じ名前のフィールドを先にする）。
/// - 入れた後で**読み直した値**を返す（代入は値を写すので、計算した値そのものを返すと 1 回目と 2 回目で別の物になる）。
/// - 文言は CPython と同じ。`__doc__` は `func.__doc__`（タスク 1-14）。
/// - ⚠ `__get__` は `mut self`（`property` と同じ理由）。
const PY_CACHED_PROPERTY_SRC: &str = r#"class cached_property:
    mut func: Any
    mut attrname: Any
    mut __doc__: Any

    fn __init__(mut self, func: Any) -> None:
        self.func = func
        self.attrname = None
        self.__doc__ = func.__doc__

    fn __set_name__(mut self, owner: Any, name: Any) -> None:
        if self.attrname is None:
            self.attrname = name
        elif name != self.attrname:
            raise TypeError("Cannot assign the same cached_property to two different names (" + repr(self.attrname) + " and " + repr(name) + ").")

    fn __get__(mut self, mut obj: Any, owner: Any = None) -> Any:
        if obj is None:
            return self
        if self.attrname is None:
            raise TypeError("Cannot use cached_property instance without calling __set_name__ on it.")
        setattr(obj, self.attrname, self.func(obj))
        return getattr(obj, self.attrname)
"#;

/// Arrow で書いた組み込みのクラスの名前（[`Interpreter::py_builtin_class`] が組み立てる）。
pub(crate) const PY_BUILTIN_CLASS_NAMES: &[&str] = &["property", "cached_property"];

impl Interpreter {
    /// Arrow で書いた組み込みのクラス（`property` / `cached_property`）の値。どれかを初めて引いたときに
    /// まとめて組み立てる（[`PY_PROPERTY_SRC`] / [`PY_CACHED_PROPERTY_SRC`]）。
    ///
    /// 自分の大域（組み込みだけ）の枠でクラスを定義するので、メソッドの本体は利用者の名前を見ない
    /// （モジュールの本体と同じ・`enter_module_frame`）。クラスの名前に修飾は付かない（`type(p).__name__` は `property`）。
    pub(crate) fn py_builtin_class(&mut self, name: &str) -> Result<Value, String> {
        if self.py_builtin_classes.is_none() {
            let mut stmts = Vec::new();
            // トレースバックのファイル名はクラスの名前（`File "<property>", line 29, in __set__`）。
            for (src, file) in [(PY_PROPERTY_SRC, "<property>"), (PY_CACHED_PROPERTY_SRC, "<cached_property>")] {
                let tokens = crate::lexer::Lexer::new(src, file).tokenize();
                stmts.extend(
                    crate::parser::Parser::new(tokens, None)
                        .parse_program()
                        .map_err(|e| format!("RuntimeError: internal — a builtin class does not parse: {e}"))?,
                );
            }
            // ⚠ Python のモジュールの中から引かれても Arrow のクラスとして定義する（`is_python` を立てない）。
            let prev_py = std::mem::replace(&mut self.in_python_module, false);
            let prev_nested = std::mem::replace(&mut self.defining_nested_class, false);
            let frame = self.enter_module_frame(&[]);
            let result = stmts.iter().try_for_each(|st| self.exec(st).map(|_| ()));
            let classes: std::collections::HashMap<String, Value> = PY_BUILTIN_CLASS_NAMES
                .iter()
                .filter_map(|n| self.scopes[0].get(n).map(|v| (n.to_string(), v.get_value())))
                .collect();
            self.leave_module_frame(frame);
            self.defining_nested_class = prev_nested;
            self.in_python_module = prev_py;
            result.map_err(|e| format!("RuntimeError: internal — a builtin class failed: {e}"))?;
            self.py_builtin_classes = Some(classes);
        }
        self.py_builtin_classes
            .as_ref()
            .and_then(|m| m.get(name).cloned())
            .ok_or_else(|| format!("RuntimeError: internal — the builtin '{name}' was not defined"))
    }
}

impl Interpreter {
    /// 組み込みのクラス `object`（python_builtins_plan.md のタスク 6-10）。初めて引いたときに作る。
    ///
    /// - フィールドもメソッドも持たない。インスタンスは写さずに共有し、同一性で比べる（[`crate::interpreter::BuiltinClass`]）。
    /// - すべての値が `object` の派生（`isinstance(x, object)` は真）なのは型の判定の特例（`ClassValue::is_a` /
    ///   `primitive_ann_matches` / [`Self::py_isinstance`]）。このクラスの振る舞いは引き継がない。
    /// - ⚠ クラスの値を呼んでも作れない（`instantiate_evaled`）。作れるのは Python のソースの `object()` だけ
    ///   （変換器が `__py_object()` にする・[`Self::py_object`]）。
    pub(crate) fn object_class(&mut self) -> std::rc::Rc<ClassValue> {
        self.object_class
            .get_or_insert_with(|| {
                std::rc::Rc::new(ClassValue {
                    builtin: crate::interpreter::BuiltinClass::Object,
                    ..ClassValue::synthetic("object", crate::interpreter::value::alloc_class_id())
                })
            })
            .clone()
    }

    /// 唯一のインスタンスしか持たない組み込みのクラスの、その値（`Ellipsis` / `NotImplemented`・タスク 6-11 / 6-12）。
    /// 初めて引いたときに作る。
    ///
    /// フィールドもメソッドも持たないクラス（`ClassValue::synthetic` に印を付けたもの）の唯一のインスタンス。
    /// 表示は `display`（`Ellipsis` / `NotImplemented`）、比べるのは種類（`values_eq` / `values_ref_eq`）。写さない
    /// （`deep_copy_value`）。
    /// ⚠ `kind` が唯一の値の種類でなければ内部の誤り（呼び出し側の取り違え）。
    pub(crate) fn builtin_singleton(&mut self, kind: crate::interpreter::BuiltinClass) -> Value {
        use crate::interpreter::BuiltinClass as B;
        let (slot, class_name) = match kind {
            B::Ellipsis => (&mut self.ellipsis, "ellipsis"),
            B::NotImplemented => (&mut self.not_implemented, "NotImplementedType"),
            B::Ordinary | B::Object => unreachable!("builtin_singleton: {kind:?} has no single instance"),
        };
        slot.get_or_insert_with(|| {
            let class = std::rc::Rc::new(ClassValue {
                builtin: kind,
                ..ClassValue::synthetic(class_name, crate::interpreter::value::alloc_class_id())
            });
            let inst = crate::interpreter::InstanceData::new_empty(class, 0);
            Value::Instance(std::rc::Rc::new(std::cell::RefCell::new(inst)))
        })
        .clone()
    }

    /// Python の `object()`（変換器が `__py_object()` にする・タスク 6-10）。文言は CPython と同じ。
    pub(crate) fn py_object(&mut self, args: &[Value]) -> Result<Value, String> {
        if !args.is_empty() {
            return Err("TypeError: object() takes no arguments".to_string());
        }
        let inst = crate::interpreter::InstanceData::new_empty(self.object_class(), 0);
        Ok(Value::Instance(std::rc::Rc::new(std::cell::RefCell::new(inst))))
    }

    /// 基底が `object` のクラス（基底を書かない・`class C(object)`）の `super().__init__(..)`（変換器が
    /// `__py_object_init(self, ..)` にする・タスク 6-10）。CPython の `object.__init__` と同じく、`self` のほかに
    /// 引数があれば `TypeError`、無ければ何もしない。
    /// ⚠ CPython は `__new__` を上書きしたクラスでは余分な引数を許すが、Python のクラスの `__new__` は対象外。
    pub(crate) fn py_object_init(&self, args: &[Value]) -> Result<Value, String> {
        match args.len() {
            0 => Err("TypeError: descriptor '__init__' of 'object' object needs an argument".to_string()),
            1 => Ok(Value::None),
            _ => Err("TypeError: object.__init__() takes exactly one argument (the instance to initialize)".to_string()),
        }
    }

    /// `bytes()` / `bytes(n)` / `bytes(iterable)` / `bytes(s, encoding[, errors])` / `bytes(b)`
    /// （python_builtins_plan.md のタスク 3-1）。文言は CPython 3.12 と同じ。
    pub(crate) fn py_bytes_ctor(&mut self, evaled: Vec<(Option<String>, Value, bool)>) -> Result<Value, String> {
        let mut source: Option<Value> = None;
        let mut encoding: Option<Value> = None;
        let mut errors: Option<Value> = None;
        let mut pos = 0;
        for (name, v, _) in evaled {
            let slot = match name.as_deref() {
                None => {
                    pos += 1;
                    match pos {
                        1 => &mut source,
                        2 => &mut encoding,
                        3 => &mut errors,
                        n => return Err(format!("TypeError: bytes() takes at most 3 arguments ({n} given)")),
                    }
                }
                Some("source") => &mut source,
                Some("encoding") => &mut encoding,
                Some("errors") => &mut errors,
                Some(other) => {
                    return Err(format!("TypeError: '{other}' is an invalid keyword argument for bytes()"))
                }
            };
            *slot = Some(v);
        }
        let text = |v: Option<Value>, what: &str| -> Result<Option<String>, String> {
            match v {
                None => Ok(None),
                Some(Value::Str(s)) => Ok(Some(s.to_string())),
                Some(other) => Err(format!(
                    "TypeError: bytes() argument '{what}' must be str, not {}",
                    crate::interpreter::ops::typecheck::runtime_type_name(&other)
                )),
            }
        };
        let encoding = text(encoding, "encoding")?;
        let errors = text(errors, "errors")?;
        let Some(source) = source else {
            if encoding.is_some() || errors.is_some() {
                return Err("TypeError: encoding without a string argument".to_string());
            }
            return Ok(Value::Bytes(std::rc::Rc::from(&[][..])));
        };
        if let Value::Str(s) = &source {
            let Some(enc) = encoding else {
                return Err("TypeError: string argument without an encoding".to_string());
            };
            let codec = crate::interpreter::ops::codecs::Codec::lookup(&enc)?;
            let out = crate::interpreter::ops::codecs::encode(s, codec, errors.as_deref().unwrap_or("strict"))?;
            return Ok(Value::Bytes(out.into()));
        }
        if encoding.is_some() || errors.is_some() {
            return Err("TypeError: encoding without a string argument".to_string());
        }
        match source {
            Value::Bytes(b) => Ok(Value::Bytes(b)),
            // `bytes(n)` は 0 が n 個（CPython の `bool` は int の派生なので `bytes(True)` も同じ）。
            Value::Int(n) => {
                if n < 0 {
                    return Err("ValueError: negative count".to_string());
                }
                Ok(Value::Bytes(vec![0u8; n as usize].into()))
            }
            Value::Bool(t) => Ok(Value::Bytes(vec![0u8; usize::from(t)].into())),
            Value::None | Value::Float(_) | Value::Complex(_, _) => Err(format!(
                "TypeError: cannot convert '{}' object to bytes",
                crate::interpreter::ops::typecheck::runtime_type_name(&source)
            )),
            other => {
                let shown = crate::interpreter::ops::typecheck::runtime_type_name(&other);
                let items = self
                    .drain_iterable(other)
                    .map_err(|_| format!("TypeError: cannot convert '{shown}' object to bytes"))?;
                let mut out = Vec::with_capacity(items.len());
                for v in items {
                    match v {
                        Value::Int(n) if (0..=255).contains(&n) => out.push(n as u8),
                        Value::Int(_) => return Err("ValueError: bytes must be in range(0, 256)".to_string()),
                        other => {
                            return Err(format!(
                                "TypeError: '{}' object cannot be interpreted as an integer",
                                crate::interpreter::ops::typecheck::runtime_type_name(&other)
                            ))
                        }
                    }
                }
                Ok(Value::Bytes(out.into()))
            }
        }
    }
}

/// Python の形の `open` の引数を Arrow の列挙の値へ写したもの（python_builtins_plan.md のタスク 5-2）。
pub(crate) struct PyOpenForm {
    /// `FileOpenMode` の値（write=0 / rewrite=1 / read=2 / make_and_write=3）。
    pub(crate) open_mode: i64,
    /// `"a"`（追記）。無ければ作り、あれば末尾から書く。
    pub(crate) append: bool,
    /// `"b"`（バイト列として読み書きする）。
    pub(crate) binary: bool,
    /// `Encoding` の値（ASCII=0 / UTF_8=1 / UTF_8_with_BOM=2）。
    pub(crate) enc_int: i64,
    /// 渡されたモードの文字列（`"r"` / `"wb"` …・`f.mode`・ファイルを Python の意味にする印・タスク 5-4）。
    pub(crate) mode: String,
}

impl PyOpenForm {
    /// 追記で、まだ無いファイルは作る（CPython の `"a"` と同じ）。あれば読み書きで開いて末尾へ。
    pub(crate) fn open_mode_for(&self, path: &str) -> i64 {
        if self.append && !std::path::Path::new(path).exists() {
            1
        } else {
            self.open_mode
        }
    }

    pub(crate) fn start_point_for(&self, path: &str) -> i64 {
        i64::from(self.append && std::path::Path::new(path).exists())
    }
}

/// `open` の引数が Python の形か（`open(path)` / `open(path, "w")` / `open(path, mode="rb", encoding="utf-8")`）を
/// 見て、そうなら Arrow の列挙の値へ写す（タスク 5-2）。Arrow の形（2 番目が `FileOpenMode` の値・
/// `open_mode=`）なら `None`。
///
/// 文言は CPython と同じ（`invalid mode: 'q'` / `binary mode doesn't take an encoding argument` …）。
/// ⚠ 受けない形は黙って意味を変えずに止める:
///   - `"r+"`: Arrow の `write` は**位置へ挿入する**（CPython は上書き）ので写せない
///   - `newline="\r"` / `"\r\n"`: Arrow は改行を変換しない。`newline=None`（既定）も変換しない
///     （CPython は Windows で `\r\n` ⇔ `\n` を変換する）
///   - UTF-8 / UTF-8-SIG / ASCII 以外の `encoding`、`errors="strict"` 以外
pub(crate) fn py_open_form(
    pos: &[Value],
    kw: &std::collections::HashMap<String, Value>,
) -> Result<Option<PyOpenForm>, String> {
    if kw.contains_key("open_mode") || !matches!(pos.get(1), None | Some(Value::Str(_))) {
        return Ok(None);
    }
    // CPython の位置引数: file, mode, buffering, encoding, errors, newline
    if pos.len() > 6 {
        return Err(format!("TypeError: open() takes at most 6 positional arguments here ({} given)", pos.len()));
    }
    for k in kw.keys() {
        if !matches!(k.as_str(), "file" | "mode" | "buffering" | "encoding" | "errors" | "newline") {
            return Err(format!("TypeError: '{k}' is an invalid keyword argument for open()"));
        }
    }
    let get = |i: usize, name: &str| pos.get(i).or_else(|| kw.get(name));
    let mode: String = match get(1, "mode") {
        None => "r".to_string(),
        Some(Value::Str(s)) => s.to_string(),
        Some(_) => return Err("TypeError: open() argument 'mode' must be str".to_string()),
    };
    let invalid = || format!("ValueError: invalid mode: '{mode}'");
    let (mut kind, mut binary, mut text, mut plus) = (None, false, false, false);
    let mut seen = String::new();
    for c in mode.chars() {
        if !"rwxabt+".contains(c) || seen.contains(c) {
            return Err(invalid());
        }
        seen.push(c);
        match c {
            'r' | 'w' | 'x' | 'a' if kind.is_some() => {
                return Err("ValueError: must have exactly one of create/read/write/append mode".to_string())
            }
            'r' | 'w' | 'x' | 'a' => kind = Some(c),
            'b' => binary = true,
            't' => text = true,
            _ => plus = true,
        }
    }
    if binary && text {
        return Err("ValueError: can't have text and binary mode at once".to_string());
    }
    let Some(kind) = kind else {
        return Err(
            "ValueError: Must have exactly one of create/read/write/append mode and at most one plus".to_string(),
        );
    };
    let (open_mode, append) = match kind {
        'r' if plus => {
            return Err(format!(
                "NotImplementedError: open() mode '{mode}' is not supported \
                 (Arrow's write inserts at the position instead of overwriting)"
            ))
        }
        'r' => (2, false),
        'w' => (1, false),
        'x' => (3, false),
        _ => (0, true),
    };
    let enc_int = match get(3, "encoding") {
        None | Some(Value::None) => 1,
        Some(Value::Str(_)) if binary => {
            return Err("ValueError: binary mode doesn't take an encoding argument".to_string())
        }
        Some(Value::Str(e)) => match e.to_ascii_lowercase().replace('_', "-").as_str() {
            "utf-8" | "utf8" => 1,
            "utf-8-sig" => 2,
            "ascii" | "us-ascii" => 0,
            _ => {
                return Err(format!(
                    "NotImplementedError: open() encoding '{e}' is not supported (UTF-8 / UTF-8-SIG / ASCII only)"
                ))
            }
        },
        Some(_) => return Err("TypeError: open() argument 'encoding' must be str or None".to_string()),
    };
    match get(4, "errors") {
        None | Some(Value::None) => {}
        Some(_) if binary => return Err("ValueError: binary mode doesn't take an errors argument".to_string()),
        Some(Value::Str(s)) if &**s == "strict" => {}
        Some(_) => {
            return Err("NotImplementedError: open() errors= other than 'strict' is not supported".to_string())
        }
    }
    match get(5, "newline") {
        None | Some(Value::None) => {}
        Some(_) if binary => return Err("ValueError: binary mode doesn't take a newline argument".to_string()),
        Some(Value::Str(s)) if s.is_empty() || &**s == "\n" => {}
        Some(Value::Str(s)) if &**s == "\r" || &**s == "\r\n" => {
            return Err(format!(
                "NotImplementedError: open() newline={:?} is not supported (Arrow does not translate newlines)",
                &**s
            ))
        }
        Some(Value::Str(s)) => return Err(format!("ValueError: illegal newline value: {s}")),
        Some(_) => return Err("TypeError: open() argument 'newline' must be str or None".to_string()),
    }
    Ok(Some(PyOpenForm { open_mode, append, binary, enc_int, mode }))
}
