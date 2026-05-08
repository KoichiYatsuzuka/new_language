// interpreter.rs — 型定義・定数・Interpreter構造体・new()/初期化
//
// サブモジュール担当:
//   interpreter/scope.rs     — スコープ管理 (push_scope / pop_scope / get_var / declare_var / assign_var)
//   interpreter/ops.rs       — 演算・比較・真偽値・表示 (is_truthy / type_name / display / apply_binop など)
//   interpreter/exec.rs      — 文の実行 (exec / exec_block / exec_scoped_block)
//   interpreter/eval.rs      — 式の評価・attr_assign (eval / attr_assign)
//   interpreter/functions.rs — 関数・ジェネレータ・オーバーロード実行
//   interpreter/classes.rs   — クラス・インスタンス管理
//   interpreter/exceptions.rs — 例外クラス構築・トレースバック
//   interpreter/templates.rs — テンプレート展開・AST置換

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Param, Stmt};

#[path = "interpreter/scope.rs"]
mod scope;
#[path = "interpreter/ops.rs"]
mod ops;
#[path = "interpreter/exec.rs"]
mod exec;
#[path = "interpreter/eval.rs"]
mod eval;
#[path = "interpreter/functions.rs"]
mod functions;
#[path = "interpreter/classes.rs"]
mod classes;
#[path = "interpreter/exceptions.rs"]
mod exceptions;
#[path = "interpreter/templates.rs"]
mod templates;

#[cfg(test)]
#[path = "interpreter/tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// Sentinel / thread-local (private to this module tree)
// ---------------------------------------------------------------------------

/// Sentinel string returned as `Err(...)` when an exception is being raised/propagated.
/// Callers check for this value to distinguish language-level raises from interpreter bugs.
pub(self) const RAISE_SENTINEL: &str = "\x00__raise__";

thread_local! {
    /// Yield collector active while a generator body is being eagerly evaluated.
    /// `None` means we are not inside a generator execution context.
    pub(self) static GENERATOR_YIELDS: RefCell<Option<Vec<Value>>> = RefCell::new(None);
}

// ---------------------------------------------------------------------------
// Exception / traceback types
// ---------------------------------------------------------------------------

/// One frame in the error traceback (one level of the call stack).
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub file: String,
    pub line: usize,
    pub col: usize,
    /// Name of the function / method / `<module>` where the raise (or propagation) happened.
    pub fn_name: String,
    /// Up-to-5 lines of source context centred on `line`, or empty when unavailable.
    pub context: String,
}

/// A language-level exception that is propagating up the call stack.
#[derive(Debug, Clone)]
pub struct RaisedError {
    /// The exception instance (always `Value::Instance` for user-raised errors).
    pub exception: Value,
    /// Frames collected as the exception propagates: index 0 = raise site (innermost),
    /// last index = outermost frame before reaching `<module>`.
    pub frames: Vec<StackFrame>,
}

// ---------------------------------------------------------------------------
// Function / Class / Instance value types
// ---------------------------------------------------------------------------

/// A generator function definition (callable; returns a Generator when invoked).
#[derive(Debug)]
pub struct GeneratorFnValue {
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// A template generator function (not yet instantiated with concrete types).
#[derive(Debug)]
pub struct TemplateGenFnValue {
    pub template_params: Vec<crate::ast::TemplateParam>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// Runtime state of an instantiated generator object.
/// Holds all eagerly-collected yielded values and the current consumption index.
#[derive(Debug)]
pub struct GeneratorState {
    pub values: Vec<Value>,
    pub index: usize,
}

/// A template function definition (not yet instantiated with concrete types).
#[derive(Debug)]
pub struct TemplateFnValue {
    pub template_params: Vec<crate::ast::TemplateParam>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// A template class definition (not yet instantiated with concrete types).
#[derive(Debug)]
pub struct TemplateClassValue {
    pub name: String,
    pub template_params: Vec<crate::ast::TemplateParam>,
    pub bases: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct FnValue {
    pub(self) params: Vec<Param>,
    pub(self) body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct ClassValue {
    pub(self) name: String,
    pub(self) bases: Vec<String>,
    /// Each method name maps to one or more overloads.
    pub(self) methods: HashMap<String, Vec<Rc<FnValue>>>,
    /// Generator methods (defined with `gen` inside a class body).
    pub(self) gen_methods: HashMap<String, Rc<GeneratorFnValue>>,
    /// Default values for `mut`/`let` instance fields: (name, default, mutable).
    pub(self) field_defaults: Vec<(String, Value, bool)>,
    /// `const` class variables shared by all instances (always immutable).
    pub(self) class_vars: HashMap<String, Value>,
    /// Declared mutability for every instance field (used when first assigning
    /// fields that have no default value, i.e., not yet in `inst.fields`).
    pub(self) field_mutability: HashMap<String, bool>,
}

#[derive(Debug)]
pub struct InstanceData {
    pub class: Rc<ClassValue>,
    /// name → (value, mutable)
    pub fields: HashMap<String, (Value, bool)>,
    /// Set to `true` when the instance was bound with `let`.
    /// All fields become immutable and methods requiring `mut self` are forbidden.
    pub immutable: bool,
}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    List(Vec<Value>),
    Function(Rc<FnValue>),
    /// Two or more overloads of the same function name in the same scope.
    OverloadedFn(Vec<Rc<FnValue>>),
    Class(Rc<ClassValue>),
    Instance(Rc<RefCell<InstanceData>>),
    /// A type value — holds a built-in type name (`int`, `str`, `float`, `bool`).
    /// User-defined class types are represented by `Value::Class`.
    Type(String),
    /// A trait value — the runtime representation of a declared trait.
    Trait(String),
    /// An uninstantiated template function (parameterized over type variables).
    TemplateFn(Rc<TemplateFnValue>),
    /// An uninstantiated template class (parameterized over type variables).
    TemplateClass(Rc<TemplateClassValue>),
    /// A generator function (callable; returns a Generator when invoked).
    GeneratorFn(Rc<GeneratorFnValue>),
    /// A template generator function (parameterized over type variables).
    TemplateGenFn(Rc<TemplateGenFnValue>),
    /// An instantiated generator: holds all eagerly-collected yielded values.
    Generator(Rc<RefCell<GeneratorState>>),
}

// ---------------------------------------------------------------------------
// Control-flow signals
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
pub enum ExecResult {
    Normal,
    Break,
    Continue,
    Return(Value),
    BlockReturn(Value),
    /// A language-level exception propagating up the call stack.
    Raise(RaisedError),
}

// ---------------------------------------------------------------------------
// Interpreter internals
// ---------------------------------------------------------------------------

pub(self) struct Var {
    pub(self) value: Value,
    pub(self) mutable: bool,
}

pub struct Interpreter {
    pub(self) scopes: Vec<HashMap<String, Var>>,
    /// filename → list of source lines (for traceback context extraction).
    pub(self) source_map: HashMap<String, Vec<String>>,
    /// Call stack: (function_name) entries pushed/popped around function execution.
    pub(self) call_stack: Vec<String>,
    /// The exception currently being handled inside an `except` block (for bare `raise`).
    pub(self) current_exception: Option<RaisedError>,
}

impl Interpreter {
    pub fn new() -> Self {
        let mut global: HashMap<String, Var> = HashMap::new();
        // Pre-define built-in type values so `int`, `str`, `float`, `bool`
        // can be used as expressions of type `type`.
        for name in ["int", "str", "float", "bool"] {
            global.insert(name.to_string(), Var { value: Value::Type(name.to_string()), mutable: false });
        }

        // Pre-register the built-in Error trait so it is accessible as a value.
        global.insert("Error".to_string(), Var { value: Value::Trait("Error".to_string()), mutable: false });

        // Register all standard exception classes.
        // Each class has: __init__(mut self, message: str), plus default fields
        // code_context/file/line/col (populated at raise time by the interpreter).
        let exception_names = [
            "Exception", "ValueError", "TypeError", "NameError", "AttributeError",
            "IndexError", "KeyError", "ZeroDivisionError", "RuntimeError",
            "StopIteration", "NotImplementedError", "OverflowError", "IOError",
            "OSError", "AssertionError", "ArithmeticError",
        ];
        for class_name in exception_names {
            let cls = Self::make_error_class(class_name);
            global.insert(class_name.to_string(), Var { value: Value::Class(cls), mutable: false });
        }

        Self {
            scopes: vec![global],
            source_map: HashMap::new(),
            call_stack: Vec::new(),
            current_exception: None,
        }
    }

    /// Register source text for a file so that tracebacks can show context lines.
    pub fn add_source_text(&mut self, filename: &str, content: &str) {
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        self.source_map.insert(filename.to_string(), lines);
    }

    /// Take the current propagating exception out of the interpreter (for top-level handling).
    pub fn take_current_exception(&mut self) -> Option<RaisedError> {
        self.current_exception.take()
    }

    /// Format a `RaisedError` into a human-readable traceback report.
    pub fn format_error_report(raised: &RaisedError) -> String {
        let mut out = String::from("Traceback (most recent call last):\n");

        // frames[0] is innermost (raise site); display outermost first.
        for frame in raised.frames.iter().rev() {
            if frame.line == 0 {
                out.push_str(&format!("  File \"{}\", in {}\n", frame.file, frame.fn_name));
            } else {
                out.push_str(&format!(
                    "  File \"{}\", line {}, col {}, in {}\n",
                    frame.file, frame.line, frame.col, frame.fn_name
                ));
            }
            if !frame.context.is_empty() {
                for line in frame.context.lines() {
                    out.push_str(&format!("    {}\n", line));
                }
            }
        }

        // Exception class name and message.
        if let Value::Instance(inst_rc) = &raised.exception {
            let inst = inst_rc.borrow();
            let class_name = &inst.class.name;
            let message = inst.fields.get("message")
                .map(|(v, _)| match v {
                    Value::Str(s) => s.clone(),
                    Value::Int(n) => n.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => "<value>".to_string(),
                })
                .unwrap_or_default();
            out.push_str(&format!("{}: {}", class_name, message));
        } else {
            out.push_str("<exception>");
        }

        out
    }
}
