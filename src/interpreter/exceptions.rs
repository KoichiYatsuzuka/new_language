// exceptions.rs — 例外クラス構築・トレースバック
// (make_error_class / get_context_lines / exc_matches)

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Param, Stmt};

use super::{Interpreter, Value, ClassValue, FnValue};

impl Interpreter {
    /// Build a `ClassValue` for a standard exception class.
    /// Fields: `message` (immutable instance field), `code_context`, `file` (str), `line`, `col` (int).
    pub(super) fn make_error_class(class_name: &str) -> Rc<ClassValue> {
        use crate::ast::Expr as E;
        // __init__ body: self.message = message
        let init_body = vec![
            Stmt::AttrAssign {
                target: E::Attr {
                    object: Box::new(E::Ident("self".to_string())),
                    attr: "message".to_string(),
                },
                value: E::Ident("message".to_string()),
            },
        ];
        let init_fn = Rc::new(FnValue {
            params: vec![
                Param { name: "self".to_string(),    mutable: true,  type_ann: None },
                Param { name: "message".to_string(), mutable: false, type_ann: Some("str".to_string()) },
            ],
            body: init_body,
        });
        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        methods.insert("__init__".to_string(), vec![init_fn]);

        // Default values for auto-populated fields (interpreter overwrites them at raise time).
        let field_defaults = vec![
            ("code_context".to_string(), Value::Str("".to_string()), true),
            ("file".to_string(),         Value::Str("".to_string()), true),
            ("line".to_string(),         Value::Int(0),              true),
            ("col".to_string(),          Value::Int(0),              true),
        ];
        let mut field_mutability: HashMap<String, bool> = HashMap::new();
        field_mutability.insert("message".to_string(),      false); // let — immutable after __init__
        field_mutability.insert("code_context".to_string(), true);
        field_mutability.insert("file".to_string(),         true);
        field_mutability.insert("line".to_string(),         true);
        field_mutability.insert("col".to_string(),          true);

        Rc::new(ClassValue {
            name: class_name.to_string(),
            bases: vec!["Error".to_string()],
            methods,
            gen_methods: HashMap::new(),
            field_defaults,
            class_vars: HashMap::new(),
            field_mutability,
        })
    }

    /// Extract up to `n` lines of source around `line` (1-based) from the source map.
    pub(super) fn get_context_lines(&self, file: &str, line: usize, n: usize) -> String {
        let lines = match self.source_map.get(file) {
            Some(l) => l,
            None => return String::new(),
        };
        if line == 0 || lines.is_empty() { return String::new(); }
        let half = n / 2;
        let start = line.saturating_sub(half + 1); // convert to 0-based with padding
        let end = (line + half).min(lines.len());
        lines[start..end].join("\n")
    }

    /// Check whether an exception instance matches an except clause's type name.
    /// Matches if the instance's class name equals `type_name`, OR if `type_name`
    /// appears in the class's `bases` list (i.e. is a parent trait/class).
    pub(super) fn exc_matches(inst_class: &Rc<ClassValue>, type_name: &str) -> bool {
        inst_class.name == type_name || inst_class.bases.contains(&type_name.to_string())
    }
}
