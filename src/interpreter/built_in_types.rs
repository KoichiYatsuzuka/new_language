// built_in_types.rs — 組み込み型・例外クラス・列挙型の初期化
//
// インタープリタ起動時にグローバルスコープへ登録する組み込み値を構築する。
// Interpreter::new() から呼ばれる register_builtin_globals がエントリポイント。
//
// 担当:
//   make_error_class          — 標準例外クラスの ClassValue を構築
//   make_primitive_wrapper_class — new_type 相当のラッパークラスを構築
//   make_builtin_enum_class   — ビルトイン enum クラスペアを構築
//   register_builtin_globals  — グローバルスコープに全組み込み値を登録

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{Expr, Param, Stmt};
use crate::token::Span;

use super::{
    async_mgr, ClassValue, FnValue, InstanceData, NamespaceData, Value, Var,
};

/// 標準例外クラス用の `ClassValue` を構築して返す。
///
/// 生成されるクラスの構造:
/// - フィールド: `message`, `code_context`, `file`, `line`, `col`（すべて let・不変）
/// - `__init__(mut self, message: str)` メソッドで `self.message = message` を実行
/// - `code_context` / `file` / `line` / `col` は raise 時にインタープリタが直接書き込む（不変フラグのまま）
///
/// - `class_name`: 生成するクラスの名前（例: `"ValueError"`, `"TypeError"`）
///
/// 戻り値: `Rc<ClassValue>` — 構築した例外クラス定義
pub(super) fn make_error_class(class_name: &str) -> Rc<ClassValue> {
    use crate::ast::Expr as E;

    // __init__ 本体: `self.message = message` を表す AST ノード
    let init_body = vec![Stmt::AttrAssign {
        target: E::Attr {
            object: Box::new(E::Ident("self".to_string())),
            attr: "message".to_string(),
            span: Span::unknown(),
        },
        value: E::Ident("message".to_string()),
    }];
    let init_fn = Rc::new(FnValue {
        name: "__init__".to_string(),
        params: vec![
            Param {
                name: "self".to_string(),
                mutable: true,
                type_ann: None,
                default: None,
            },
            Param {
                name: "message".to_string(),
                mutable: false,
                type_ann: Some("str".to_string()),
                default: None,
            },
        ],
        body: init_body,
        is_python: false,
        captured_env: HashMap::new(),
    });
    let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
    methods.insert("__init__".to_string(), vec![init_fn]);

    // raise 時にインタープリタが自動上書きするフィールドのデフォルト値（空文字・0で初期化）
    let field_defaults = vec![
        ("code_context".to_string(), Value::Str("".to_string()), false),
        ("file".to_string(), Value::Str("".to_string()), false),
        ("line".to_string(), Value::Int(0), false),
        ("col".to_string(), Value::Int(0), false),
    ];

    // フィールドの可変フラグ: `message` は `let`（__init__ 後は不変）、他は `mut`（可変）
    let mut field_mutability: HashMap<String, bool> = HashMap::new();
    field_mutability.insert("message".to_string(), false);
    field_mutability.insert("code_context".to_string(), false);
    field_mutability.insert("file".to_string(), false);
    field_mutability.insert("line".to_string(), false);
    field_mutability.insert("col".to_string(), false);

    Rc::new(ClassValue {
        name: class_name.to_string(),
        bases: vec!["Error".to_string()],
        methods,
        gen_methods: HashMap::new(),
        field_defaults,
        class_vars: HashMap::new(),
        field_mutability,
        field_access: HashMap::new(),
        method_access: HashMap::new(),
        static_method_names: HashSet::new(),
        class_method_names: HashSet::new(),
        static_vars: HashMap::new(),
        new_type_base: None,
    })
}

/// `new_type <name>: <prim_type>` 相当のラッパークラスを生成する。
/// 生成クラスは `mut value: <prim_type>` フィールドと `__init__(mut self, value: <prim_type>)` を持つ。
pub(super) fn make_primitive_wrapper_class(name: &str, prim_type: &str) -> Rc<ClassValue> {
    let init_body = vec![Stmt::AttrAssign {
        target: Expr::Attr {
            object: Box::new(Expr::Ident("self".to_string())),
            attr: "value".to_string(),
            span: Span::unknown(),
        },
        value: Expr::Ident("value".to_string()),
    }];
    let init_fn = Rc::new(FnValue {
        name: "__init__".to_string(),
        params: vec![
            Param {
                name: "self".to_string(),
                mutable: true,
                type_ann: None,
                default: None,
            },
            Param {
                name: "value".to_string(),
                mutable: false,
                type_ann: Some(prim_type.to_string()),
                default: None,
            },
        ],
        body: init_body,
        is_python: false,
        captured_env: HashMap::new(),
    });
    let mut methods = HashMap::new();
    methods.insert("__init__".to_string(), vec![init_fn]);
    Rc::new(ClassValue {
        name: name.to_string(),
        bases: vec![],
        methods,
        gen_methods: HashMap::new(),
        field_defaults: vec![],
        class_vars: HashMap::new(),
        field_mutability: HashMap::from([("value".to_string(), true)]),
        field_access: HashMap::new(),
        method_access: HashMap::new(),
        static_method_names: HashSet::new(),
        class_method_names: HashSet::new(),
        static_vars: HashMap::new(),
        new_type_base: None,
    })
}

/// ビルトイン enum クラスのペア（item クラス + enum クラス）を Rust コードで生成する。
///
/// - `name`: enum クラス名（例: `"FileOpenMode"`）
/// - `variants`: (バリアント名, 整数値) のスライス
///
/// 戻り値: `(item_cls_name, item_cls, enum_cls)` のタプル
pub(super) fn make_builtin_enum_class(
    name: &str,
    variants: &[(&str, i64)],
) -> (String, Rc<ClassValue>, Rc<ClassValue>) {
    let item_cls_name = format!("enum_item_{name}");
    // バリアントのインスタンス型（`enum_item_X`）: value フィールドを持つだけ
    let item_cls = Rc::new(ClassValue {
        name: item_cls_name.clone(),
        bases: vec![],
        methods: HashMap::new(),
        gen_methods: HashMap::new(),
        field_defaults: vec![],
        class_vars: HashMap::new(),
        field_mutability: HashMap::from([("value".to_string(), true)]),
        field_access: HashMap::new(),
        method_access: HashMap::new(),
        static_method_names: HashSet::new(),
        class_method_names: HashSet::new(),
        static_vars: HashMap::new(),
        new_type_base: None,
    });
    // 各バリアントをインスタンスとして生成し class_vars に登録
    let mut class_vars: HashMap<String, Value> = HashMap::new();
    for (variant_name, int_val) in variants {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), (Value::Int(*int_val), true));
        let inst = Value::Instance(Rc::new(RefCell::new(InstanceData {
            class: item_cls.clone(),
            fields,
            immutable: false,
        })));
        class_vars.insert(variant_name.to_string(), inst);
    }
    // enum クラス本体（バリアントのみ保持、インスタンス化不可）
    let enum_cls = Rc::new(ClassValue {
        name: name.to_string(),
        bases: vec![],
        methods: HashMap::new(),
        gen_methods: HashMap::new(),
        field_defaults: vec![],
        class_vars,
        field_mutability: HashMap::new(),
        field_access: HashMap::new(),
        method_access: HashMap::new(),
        static_method_names: HashSet::new(),
        class_method_names: HashSet::new(),
        static_vars: HashMap::new(),
        new_type_base: None,
    });
    (item_cls_name, item_cls, enum_cls)
}

/// インタープリタのグローバルスコープに全組み込み値を登録する。
/// `Interpreter::new()` から呼ばれる。
pub(super) fn register_builtin_globals(global: &mut HashMap<String, Var>) {
    // 組み込み型値を事前定義: `int`, `str`, `float`, `bool`, `dict`, `function`, `slice` を型式として使えるようにする
    // `len` も `Value::Type` として登録しておく — ネイティブコードが cb_get_global("len") で取得して
    // call_value_with_args 経由で呼べるようにするため。
    for name in [
        "int", "uint", "str", "float", "bool", "dict", "set", "function", "len", "slice",
    ] {
        global.insert(
            name.to_string(),
            Var::new(Value::Type(name.to_string()), false),
        );
    }

    // `pointer` は `new_type pointer: uint` 相当のラッパークラスとして事前登録する。
    global.insert(
        "pointer".to_string(),
        Var::new(
            Value::Class(make_primitive_wrapper_class("pointer", "uint")),
            false,
        ),
    );

    // `id` 組み込み関数: 任意のオブジェクトの同一性を表す pointer 値を返す。
    global.insert(
        "id".to_string(),
        Var::new(Value::Type("id".to_string()), false),
    );

    // 組み込み `Error` trait を事前登録（値としてアクセス可能にする）
    global.insert(
        "Error".to_string(),
        Var::new(Value::Trait("Error".to_string()), false),
    );

    // 標準例外クラスをすべて登録する。
    // 各クラスは `__init__(mut self, message: str)` を持ち、
    // code_context / file / line / col フィールドは raise 時にインタープリタが設定する。
    let exception_names = [
        "Exception",
        "ValueError",
        "TypeError",
        "NameError",
        "AttributeError",
        "IndexError",
        "KeyError",
        "ZeroDivisionError",
        "RuntimeError",
        "StopIteration",
        "NotImplementedError",
        "OverflowError",
        "IOError",
        "OSError",
        "AssertionError",
        "ArithmeticError",
        "AccessError",
    ];
    for class_name in exception_names {
        let cls = make_error_class(class_name);
        global.insert(class_name.to_string(), Var::new(Value::Class(cls), false));
    }

    // 組み込み new_type ラッパークラスを登録する
    // `path`: new_type path: str 相当、`Size`: new_type Size: int 相当
    for (cls_name, prim_type) in [("path", "str"), ("Size", "int")] {
        global.insert(
            cls_name.to_string(),
            Var::new(
                Value::Class(make_primitive_wrapper_class(cls_name, prim_type)),
                false,
            ),
        );
    }

    // Index クラスを先に生成し、begin / last 定数のインスタンス生成に再利用する
    let index_cls = make_primitive_wrapper_class("Index", "int");
    global.insert(
        "Index".to_string(),
        Var::new(Value::Class(index_cls.clone()), false),
    );

    // 組み込み定数: begin = Index(0)、last = Index(-1)
    for (const_name, int_val) in [("begin", 0i64), ("last", -1i64)] {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), (Value::Int(int_val), true));
        let inst = Value::Instance(Rc::new(RefCell::new(InstanceData {
            class: index_cls.clone(),
            fields,
            immutable: false,
        })));
        global.insert(const_name.to_string(), Var::new(inst, false));
    }

    // ファイル I/O 組み込み列挙型を登録する
    for (enum_name, variants) in [
        (
            "FileOpenMode",
            vec![
                ("write", 0i64),
                ("rewrite", 1),
                ("read", 2),
                ("make_and_write", 3),
            ],
        ),
        ("StartPoint", vec![("top", 0), ("end", 1)]),
        ("ByteRecognizingMode", vec![("byte", 0), ("text", 1)]),
        (
            "Encoding",
            vec![
                ("ASCII", 0),
                ("UTF_8", 1),
                ("UTF_8_with_BOM", 2),
                ("shift_JIS", 3),
            ],
        ),
    ] {
        let (item_name, item_cls, enum_cls) =
            make_builtin_enum_class(enum_name, &variants);
        global.insert(item_name, Var::new(Value::Class(item_cls), false));
        global.insert(
            enum_name.to_string(),
            Var::new(Value::Class(enum_cls), false),
        );
    }

    // AsyncManager: built-in constructor callable as AsyncManager(num_thread=N)
    global.insert(
        "AsyncManager".to_string(),
        Var::new(Value::Type("AsyncManager".to_string()), false),
    );

    // Async namespace: Async.Waiting / Async.Running / Async.Done
    {
        let mut members = HashMap::new();
        members.insert(
            "Waiting".to_string(),
            Value::AsyncStatusVal(async_mgr::AsyncStatus::Waiting),
        );
        members.insert(
            "Running".to_string(),
            Value::AsyncStatusVal(async_mgr::AsyncStatus::Running),
        );
        members.insert(
            "Done".to_string(),
            Value::AsyncStatusVal(async_mgr::AsyncStatus::Done),
        );
        global.insert(
            "Async".to_string(),
            Var::new(
                Value::Namespace(Rc::new(NamespaceData {
                    name: "Async".to_string(),
                    members,
                })),
                false,
            ),
        );
    }
}
