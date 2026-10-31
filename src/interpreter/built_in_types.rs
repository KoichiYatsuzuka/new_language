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
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Expr, Param, Stmt, Resolution};
use crate::token::Span;

use super::{
    async_mgr, ClassValue, FnValue, InstanceData, NamespaceData, Value, Var,
};

/// `OSError` の系統だけが持つ欄（CPython の `OSError` の属性・known_bugs.md の #9）。⚠ Windows の `winerror` は持たない。
pub(crate) const OS_ERROR_FIELDS: [&str; 4] = ["errno", "strerror", "filename", "filename2"];

/// errno に対応する `OSError` の派生（CPython の `errnomap`。`OSError(2, "..")` は `FileNotFoundError` を作る・
/// known_bugs.md の #9）。値は CPython の `errno` モジュールと同じで、OS ごとに違う番号は分けてある
/// （Windows のネットワークの誤りは WinSock の番号。Windows の CPython 3.12 で全番号を作って確かめた）。
pub(crate) fn os_error_subclass_of(errno: i64) -> Option<&'static str> {
    // どの OS でも同じ番号（EPERM / ENOENT / ESRCH / EINTR / ECHILD / EACCES / EEXIST / ENOTDIR / EISDIR / EPIPE）。
    const COMMON: &[(i64, &str)] = &[
        (1, "PermissionError"),
        (2, "FileNotFoundError"),
        (3, "ProcessLookupError"),
        (4, "InterruptedError"),
        (10, "ChildProcessError"),
        (13, "PermissionError"),
        (17, "FileExistsError"),
        (20, "NotADirectoryError"),
        (21, "IsADirectoryError"),
        (32, "BrokenPipeError"),
    ];
    // EAGAIN / EWOULDBLOCK / EINPROGRESS / EALREADY・ESHUTDOWN・ECONNABORTED・ECONNREFUSED・ECONNRESET・ETIMEDOUT
    #[cfg(windows)]
    const PLATFORM: &[(i64, &str)] = &[
        (11, "BlockingIOError"),
        (10035, "BlockingIOError"),
        (10036, "BlockingIOError"),
        (10037, "BlockingIOError"),
        (10058, "BrokenPipeError"),
        (10053, "ConnectionAbortedError"),
        (10061, "ConnectionRefusedError"),
        (10054, "ConnectionResetError"),
        (10060, "TimeoutError"),
    ];
    #[cfg(target_os = "macos")]
    const PLATFORM: &[(i64, &str)] = &[
        (35, "BlockingIOError"),
        (36, "BlockingIOError"),
        (37, "BlockingIOError"),
        (58, "BrokenPipeError"),
        (53, "ConnectionAbortedError"),
        (61, "ConnectionRefusedError"),
        (54, "ConnectionResetError"),
        (60, "TimeoutError"),
    ];
    #[cfg(not(any(windows, target_os = "macos")))]
    const PLATFORM: &[(i64, &str)] = &[
        (11, "BlockingIOError"),
        (114, "BlockingIOError"),
        (115, "BlockingIOError"),
        (108, "BrokenPipeError"),
        (103, "ConnectionAbortedError"),
        (111, "ConnectionRefusedError"),
        (104, "ConnectionResetError"),
        (110, "TimeoutError"),
    ];
    COMMON.iter().chain(PLATFORM).find(|(n, _)| *n == errno).map(|(_, c)| *c)
}

/// 標準例外クラス用の `ClassValue` を構築して返す。
///
/// 生成されるクラスの構造:
/// - フィールド: `message`, `code_context`, `file`, `line`, `col`, `args`（すべて let・不変）
/// - **`__init__` は持たない**（タスク 4-2）。作るときの位置引数を `args` に、CPython の `str(e)` を
///   `message` に入れるのは `instantiate_evaled`（CPython の `BaseException.__new__` と同じ位置）、
///   `super().__init__(..)` / `ValueError.__init__(self, ..)` は `eval_class_method` が受ける
///   （どちらも `Interpreter::exc_set_args`）。以前は `__init__(mut self, message: str)` で、
///   `ValueError()` / `ValueError("a", 1)` が作れず、`super().__init__(..)` も届かなかった。
/// - `code_context` / `file` / `line` / `col` は raise 時にインタープリタが直接書き込む（不変フラグのまま）
/// - `OSError` の系統（別名・派生を含む）はさらに `errno` / `strerror` / `filename` / `filename2` を持つ
///   （既定は `None`・CPython と同じく書き換えられる・known_bugs.md の #9）。入れるのは `exc_set_args`
///   （`OSError(errno, strerror[, filename])`）と、実行時の誤りの控え（`open` の失敗・`make_internal_raised_error`）
///
/// - `class_name`: 生成するクラスの名前（例: `"ValueError"`, `"TypeError"`）
/// - `bases`: `Error` trait と祖先を平らに並べたもの（`names::builtin_exception_bases`・タスク 4-1）
///
/// 戻り値: `Rc<ClassValue>` — 構築した例外クラス定義
pub(super) fn make_error_class(class_name: &str, bases: Vec<String>) -> Rc<ClassValue> {
    // raise 時にインタープリタが自動上書きするフィールドのデフォルト値（空文字・0で初期化）。
    // `message` / `args` は `instantiate_evaled` が入れ直す（ここは作り方を問わない既定）。
    let mut field_defaults = vec![
        ("message".to_string(), Value::str(String::new()), false),
        ("code_context".to_string(), Value::str("".to_string()), false),
        ("file".to_string(), Value::str("".to_string()), false),
        ("line".to_string(), Value::Int(0), false),
        ("col".to_string(), Value::Int(0), false),
        ("args".to_string(), empty_tuple(), false),
    ];
    let os_group = class_name == "OSError" || bases.iter().any(|b| b == "OSError");
    if os_group {
        for name in OS_ERROR_FIELDS {
            field_defaults.push((name.to_string(), Value::None, true));
        }
    }

    // フィールドの可変フラグ: `OSError` の欄のほかは `let`（不変）
    // フィールドレイアウト: message=0, code_context=1, file=2, line=3, col=4, args=5（`OSError` は errno=6 …）
    let field_mutability: HashMap<String, bool> =
        field_defaults.iter().map(|(name, _, mutable)| (name.clone(), *mutable)).collect();
    let field_mutability_vec: Vec<bool> = field_defaults.iter().map(|(_, _, mutable)| *mutable).collect();

    // field_index: own フィールド + Error:: trait エイリアス（`args` は `Error` trait に無い）
    let mut field_index: HashMap<String, usize> = [
        ("message", 0usize),
        ("code_context", 1),
        ("file", 2),
        ("line", 3),
        ("col", 4),
        ("args", 5),
        ("Error::code_context", 1),
        ("Error::file", 2),
        ("Error::line", 3),
        ("Error::col", 4),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect();
    if os_group {
        for (i, name) in OS_ERROR_FIELDS.iter().enumerate() {
            field_index.insert(name.to_string(), 6 + i);
        }
    }

    Rc::new(ClassValue {
        bases,
        methods: HashMap::new(),
        field_count: field_defaults.len(),
        field_defaults,
        field_mutability,
        field_index,
        field_mutability_vec,
        is_exception: true,
        ..ClassValue::synthetic(class_name.to_string(), crate::interpreter::value::alloc_class_id())
    })
}

/// `"ValueError: …"` の形の内部エラーなら、その組み込みの例外クラスの名前（7-4）。
/// `make_internal_raised_error` がこれで例外に変え、外部言語のブリッジはこれで「もう種類が付いたエラー」を見分ける。
///
/// 表は組み込みの例外クラスの表 1 つ（`type_check::names::BUILTIN_EXCEPTIONS`・python_builtins_plan.md のタスク 4-1）。
/// ⚠ 以前は手で揃えた別の一覧（`CATCHABLE`・のちに `BUILTIN_EXCEPTION_NAMES`）で、`RecursionError` が抜けて
///   `except RecursionError:` で捕まえられなかった（実測）・`FfiTypeError` も一度抜けていた。
/// ⚠ `SyntaxError` の系統は変換しない。内部で `SyntaxError: 'break' outside for/while loop` などを
///   返すが、CPython ではコンパイル時の誤りで `try` では捕まらない（`except Exception` に黙って
///   飲まれないようにする）。
pub(crate) fn builtin_exception_of(msg: &str) -> Option<&'static str> {
    crate::type_check::names::BUILTIN_EXCEPTIONS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !matches!(*n, "SyntaxError" | "IndentationError" | "TabError"))
        .find(|name| {
            msg.starts_with(name) && matches!(msg.as_bytes().get(name.len()), None | Some(b':') | Some(b' '))
        })
}

/// 空のタプル（例外の `args` の既定）。
pub(super) fn empty_tuple() -> Value {
    Value::Tuple(Rc::new(super::TupleData::new(Vec::new(), Vec::new())))
}

/// `new_type <name>: <prim_type>` 相当のラッパークラスを生成する。
/// 生成クラスは `mut value: <prim_type>` フィールドと `__init__(mut self, value: <prim_type>)` を持つ。
pub(super) fn make_primitive_wrapper_class(name: &str, prim_type: &str) -> Rc<ClassValue> {
    let init_body = vec![Stmt::AttrAssign {
        target: Expr::Attr {
            object: Box::new(Expr::Ident { name: "self".to_string(), node_id: 0, res: Resolution::Unresolved }),
            attr: "value".to_string(),
            span: Span::unknown(),
            cache: Default::default(),
            node_id: 0, // #16: 合成コード（注釈対象外）
        },
        value: Expr::Ident { name: "value".to_string(), node_id: 0, res: Resolution::Unresolved },
        span: crate::token::Span::unknown(),
    }];
    let init_fn = Rc::new(FnValue {
        globals: crate::interpreter::value::GLOBALS_OF_CALLER,
        owner_class: None,
        name: "__init__".to_string(),
        params: vec![
            Param {
                name: "self".to_string(),
                mutable: true,
                type_ann: None,
                default: None,
                variadic: false,
            },
            Param {
                name: "value".to_string(),
                mutable: false,
                type_ann: Some(prim_type.to_string()),
                default: None,
                variadic: false,
            },
        ],
        body: std::rc::Rc::from(init_body),
        is_python: false,
        captured_env: HashMap::new(),
    return_type: None,
    vm_chunk: None,
    bound: None,
    qualname: None,
    });
    let mut methods = HashMap::new();
    methods.insert("__init__".to_string(), vec![init_fn]);
    Rc::new(ClassValue {
        methods,
        field_mutability: HashMap::from([("value".to_string(), true)]),
        field_index: HashMap::from([("value".to_string(), 0usize)]),
        field_count: 1,
        field_mutability_vec: vec![true],
        ..ClassValue::synthetic(name.to_string(), crate::interpreter::value::alloc_class_id())
    })
}

/// ビルトイン enum を Rust コードで生成し、`Name` クラスを返す。
///
/// - `name`: enum クラス名（例: `"FileOpenMode"`）
/// - `variants`: (バリアント名, 整数値) のスライス
///
/// メンバーのクラスは `Interpreter::build_enum_classes` と同じ形（名前は enum と同じ `Name`・
/// `value` フィールドを 1 つ・`enum_of` が `Some(Name)`）。名前では宣言しない
/// （`enum_item_Name` は廃止した名前・`implementation_plans/enum_member_type_plan.md` 1-2）。
pub(super) fn make_builtin_enum_class(name: &str, variants: &[(&str, i64)]) -> Rc<ClassValue> {
    // メンバーのクラス: value フィールドを持つだけ。⚠ `value` は不変（メンバーは値・`build_enum_classes`）
    let item_cls_id = crate::interpreter::value::alloc_class_id();
    let item_cls = Rc::new(ClassValue {
        field_mutability: HashMap::from([("value".to_string(), false)]),
        field_index: HashMap::from([("value".to_string(), 0usize)]),
        field_count: 1,
        field_mutability_vec: vec![false],
        enum_of: Some(name.to_string()),
        ..ClassValue::synthetic(name.to_string(), item_cls_id)
    });
    // 各バリアントをインスタンスとして生成し class_vars に登録
    let mut class_vars: HashMap<String, Value> = HashMap::new();
    for (variant_name, int_val) in variants {
        let mut data = InstanceData::new_empty(item_cls.clone(), 0);
        data.store_field(0, Value::Int(*int_val), false);
        class_vars.insert(variant_name.to_string(), Value::Instance(Rc::new(RefCell::new(data))));
    }
    // enum クラス本体（バリアントのみ保持、インスタンス化不可）
    Rc::new(ClassValue {
        class_vars,
        ..ClassValue::synthetic(name.to_string(), crate::interpreter::value::alloc_class_id())
    })
}

/// インタープリタのグローバルスコープに全組み込み値を登録する。
/// `Interpreter::new()` から呼ばれる。
pub(super) fn register_builtin_globals(global: &mut super::ScopeMap) {
    // 組み込み型値を事前定義: `int`, `str`, `float`, `bool`, `dict`, `function`, `slice` を型式として使えるようにする
    // `len` も `Value::Type` として登録しておく — ネイティブコードが cb_get_global("len") で取得して
    // call_value_with_args 経由で呼べるようにするため。
    for name in [
        "int", "uint", "str", "bytes", "float", "complex", "bool", "dict", "set", "function", "len", "slice",
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

    // 標準例外クラスをすべて登録する（表は `type_check::names::BUILTIN_EXCEPTIONS`・タスク 4-1）。
    // 各クラスは `__init__(mut self, message: str)` を持ち、
    // code_context / file / line / col フィールドは raise 時にインタープリタが設定する。
    for (class_name, _) in crate::type_check::names::BUILTIN_EXCEPTIONS {
        let bases = crate::type_check::names::builtin_exception_bases(class_name, true);
        let cls = make_error_class(class_name, bases);
        global.insert(class_name.to_string(), Var::new(Value::Class(cls), false));
    }

    // 組み込み new_type ラッパークラスを登録する
    // `path`: new_type path: str 相当、`Size`: new_type Size: int 相当
    // ⚠ `Path` は 7-25 で `path` から改名した（利用者の決定。組み込みの型名は大域を占めるので、`path` だと
    //   `let path = ..` が書けなかった）
    for (cls_name, prim_type) in [("Path", "str"), ("Size", "int")] {
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
        let mut data = InstanceData::new_empty(index_cls.clone(), 0);
        data.store_field(0, Value::Int(int_val), true);
        let inst = Value::Instance(Rc::new(RefCell::new(data)));
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
        let enum_cls = make_builtin_enum_class(enum_name, &variants);
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

    // Result 型コンストラクタ: Ok(value) / Err(error)
    for name in ["Ok", "Err"] {
        global.insert(name.to_string(), Var::new(Value::Type(name.to_string()), false));
    }

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
                    live: None,
                })),
                false,
            ),
        );
    }
}
