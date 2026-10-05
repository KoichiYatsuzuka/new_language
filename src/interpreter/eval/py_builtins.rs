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
