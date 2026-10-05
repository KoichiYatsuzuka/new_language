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
