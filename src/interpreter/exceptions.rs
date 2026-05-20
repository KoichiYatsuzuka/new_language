// exceptions.rs — 例外クラス構築・トレースバック
// (make_error_class / get_context_lines / exc_matches)
//
// 標準例外クラスの `ClassValue` を構築するファクトリ関数と、
// トレースバック表示のためのソースコンテキスト抽出・例外マッチング判定を提供する。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Param, Stmt};

use super::{Interpreter, Value, ClassValue, FnValue};

impl Interpreter {
    /// 標準例外クラス用の `ClassValue` を構築して返す。
    ///
    /// 生成されるクラスの構造:
    /// - フィールド: `message`（let・不変）, `code_context`, `file`, `line`, `col`（mut・可変）
    /// - `__init__(mut self, message: str)` メソッドで `self.message = message` を実行
    /// - `code_context` / `file` / `line` / `col` は raise 時にインタープリタが上書きする
    ///
    /// - `class_name`: 生成するクラスの名前（例: `"ValueError"`, `"TypeError"`）
    ///
    /// 戻り値: `Rc<ClassValue>` — 構築した例外クラス定義
    pub(super) fn make_error_class(class_name: &str) -> Rc<ClassValue> {
        use crate::ast::Expr as E;

        // __init__ 本体: `self.message = message` を表す AST ノード
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
            name: "__init__".to_string(),
            params: vec![
                Param { name: "self".to_string(),    mutable: true,  type_ann: None, default: None },
                Param { name: "message".to_string(), mutable: false, type_ann: Some("str".to_string()), default: None },
            ],
            body: init_body,
            is_python: false,
            captured_env: std::collections::HashMap::new(),
        });
        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        methods.insert("__init__".to_string(), vec![init_fn]);

        // raise 時にインタープリタが自動上書きするフィールドのデフォルト値（空文字・0で初期化）
        let field_defaults = vec![
            ("code_context".to_string(), Value::Str("".to_string()), true),
            ("file".to_string(),         Value::Str("".to_string()), true),
            ("line".to_string(),         Value::Int(0),              true),
            ("col".to_string(),          Value::Int(0),              true),
        ];

        // フィールドの可変フラグ: `message` は `let`（__init__ 後は不変）、他は `mut`（可変）
        let mut field_mutability: HashMap<String, bool> = HashMap::new();
        field_mutability.insert("message".to_string(),      false); // let — __init__ 後は不変
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
            field_access: HashMap::new(),
            method_access: HashMap::new(),
            static_method_names: std::collections::HashSet::new(),
            class_method_names: std::collections::HashSet::new(),
            static_vars: HashMap::new(),
            new_type_base: None,
        })
    }

    /// ソースマップから指定行の前後 `n` 行以内のソースコンテキストを返す。
    ///
    /// - `file`: ソースファイル名（`source_map` のキーと一致させること）
    /// - `line`: 中心とする行番号（1始まり）
    /// - `n`: 取得する行数の上限（中心行の前後を含む）
    ///
    /// 戻り値: 該当行が存在する場合は複数行の文字列（`\n` 区切り）、存在しない場合は空文字列
    pub(super) fn get_context_lines(&self, file: &str, line: usize, n: usize) -> String {
        let lines = match self.source_map.get(file) {
            Some(l) => l,
            None => return String::new(),
        };
        if line == 0 || lines.is_empty() { return String::new(); }
        let half = n / 2;
        // 0始まりインデックスに変換してパディング付きで範囲を決定する
        let start = line.saturating_sub(half + 1);
        let end = (line + half).min(lines.len());
        lines[start..end].join("\n")
    }

    /// 例外インスタンスのクラスが `except` 節の型名にマッチするか判定する。
    ///
    /// マッチ条件: クラス名が `type_name` と一致するか、または `bases`（基底クラス・trait）に含まれる場合。
    ///
    /// - `inst_class`: 例外インスタンスのクラス定義
    /// - `type_name`: `except` 節で指定された型名
    ///
    /// 戻り値: `true` — マッチあり（例外がこの handler で捕捉される）
    pub(super) fn exc_matches(inst_class: &Rc<ClassValue>, type_name: &str) -> bool {
        inst_class.name == type_name || inst_class.bases.contains(&type_name.to_string())
    }
}
