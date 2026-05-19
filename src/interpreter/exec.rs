// exec.rs — 文の実行 (exec / exec_block / exec_scoped_block)
//
// `Interpreter::exec` が文（`Stmt`）を再帰的にツリーウォークして `ExecResult` を返す。
// 変数宣言・代入・制御構造・関数/クラス定義・例外処理など、すべての文の実行を担当する。

use std::cell::RefCell;
use std::rc::Rc;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::{Accessibility, Expr, FieldKind, MatchPattern, Param, Stmt};

use super::{
    CapturedVar, Interpreter, Value, Var, ExecResult,
    FnValue, TemplateFnValue, GeneratorFnValue, TemplateGenFnValue, TemplateClassValue,
    GeneratorState, NamespaceData, ModuleState, NativeFnRef, NativeLibWrapper,
    RaisedError, StackFrame,
    RAISE_SENTINEL, GENERATOR_YIELDS, BLOCK_YIELDS, LOOP_DEPTH,
};

impl Interpreter {
    /// 文（`Stmt`）を実行して `ExecResult` を返す。インタープリタの文実行のメインエントリポイント。
    ///
    /// 各 `Stmt` バリアントを処理する:
    /// - `Let` / `Const` / `Mut`: 変数を宣言してスコープに追加
    /// - `Assign` / `CompoundAssign`: 既存変数への代入（可変性チェック付き）
    /// - `AttrAssign` / `AttrCompoundAssign`: インスタンスフィールド・辞書添字への代入
    /// - `If` / `While` / `For` / `Block`: 制御構造の実行
    /// - `FnDef` / `GenDef`: 関数・ジェネレータ定義をスコープに登録（オーバーロード蓄積）
    /// - `ClassDef` / `TraitDef` / `NewTypeDef`: クラス・trait・new_type をスコープに登録
    /// - `Return` / `Break` / `Continue` / `BlockReturn` / `LoopYield`: 制御フロー信号を返す
    /// - `Yield`: スレッドローカルの yield コレクタに値を追加
    /// - `Raise` / `Try`: 例外の発生と捕捉
    /// - `Freeze`: インスタンスを不変化
    ///
    /// - `stmt`: 実行する文の AST ノード
    ///
    /// 戻り値: `Ok(ExecResult)` — 実行結果（制御フロー信号を含む）。`Err(message)` — ランタイムエラー
    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        match stmt {
            Stmt::Expr(expr) => {
                // 式文: 式を評価して副作用を実行し、値は捨てる
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, expr) => {
                // mut → let: deep copy してからフリーズプロトコルを適用する。
                // let → let: そのまま代入（コピー不要・再フリーズ不要）。
                // 式 → let: フリーズのみ（新規値なのでコピー不要）。
                let source_var = if let Expr::Ident(src) = expr {
                    self.get_var(src).map(|v| (v.mutable, v.mutable_cell.is_some()))
                } else {
                    None
                };
                let value = self.eval(expr)?;
                let value = match source_var {
                    Some((true, _)) => {
                        // mut → let: deep copy + freeze
                        let copied = Self::deep_copy_value(value);
                        self.apply_freeze_to_value(&copied)?;
                        copied
                    }
                    Some((false, _)) => {
                        // let → let: 既にフリーズ済みの値を共有するだけ
                        value
                    }
                    None => {
                        // 式 → let: 新規値をフリーズしてバインド
                        self.apply_freeze_to_value(&value)?;
                        value
                    }
                };
                self.declare_var(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::Const(name, expr) => {
                // 定数宣言: 常に不変として登録する
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, expr) => {
                // いずれの代入でも独立したコピーを持つよう deep copy する。
                // let → mut / mut → mut のどちらも対象。
                let value = self.eval(expr)?;
                let value = Self::deep_copy_value(value);
                self.declare_var(name.clone(), Var::new(value, true));
                Ok(ExecResult::Normal)
            }
            Stmt::Static(name, expr, span) => {
                // static mut 変数宣言: 宣言位置をキーとして永続セルを取得・生成する
                let key = (span.file.to_string(), span.line, span.col);
                let cell = if let Some(existing) = self.static_cells.get(&key) {
                    existing.clone()
                } else {
                    let value = self.eval(expr)?;
                    let new_cell = Rc::new(RefCell::new(value));
                    self.static_cells.insert(key, new_cell.clone());
                    new_cell
                };
                self.declare_var(name.clone(), Var::new_cell(cell));
                Ok(ExecResult::Normal)
            }
            Stmt::Assign { name, value, .. } => {
                // 変数への代入: assign_var で可変性チェックを行う
                let value = self.eval(value)?;
                self.assign_var(name, value)?;
                Ok(ExecResult::Normal)
            }
            Stmt::AttrAssign { target, value } => {
                // 属性への代入: `self.field = value` や `d[key] = value` など
                let rhs = self.eval(value)?;
                self.attr_assign(target, rhs)?;
                Ok(ExecResult::Normal)
            }
            Stmt::AttrCompoundAssign { target, op, value } => {
                // 属性への複合代入: `self.field += value` など（現在値を取得して演算後に代入）
                let rhs = self.eval(value)?;
                let lhs = self.eval(target)?;
                let result = self.apply_binop(op, lhs, rhs)?;
                self.attr_assign(target, result)?;
                Ok(ExecResult::Normal)
            }
            Stmt::CompoundAssign { name, op, value, .. } => {
                // 変数への複合代入: `x += value` など（可変性チェック付き）
                let rhs = self.eval(value)?;
                let lhs = match self.get_var(name) {
                    Some(v) if !v.mutable => {
                        return Err(format!(
                            "TypeError: cannot assign to immutable variable '{name}'"
                        ));
                    }
                    Some(v) => v.get_value(),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                };
                let value = self.apply_binop(op, lhs, rhs)?;
                self.assign_var(name, value)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Field { .. } => Ok(ExecResult::Normal), // クラス本体内でのみ有効（exec では何もしない）
            Stmt::Break => {
                // break は for/while ループ内でのみ有効。ループ外で使用すると実行時エラー。
                let in_loop = LOOP_DEPTH.with(|d| *d.borrow() > 0);
                if !in_loop {
                    return Err("SyntaxError: 'break' outside for/while loop".to_string());
                }
                Ok(ExecResult::BlockReturn(Value::None))
            }
            Stmt::Continue => Ok(ExecResult::Continue),
            Stmt::Return(expr) => {
                // return 文: 式があれば評価して Return シグナルとして返す
                let val = match expr {
                    Some(e) => self.eval(e)?,
                    None => Value::None,
                };
                Ok(ExecResult::Return(val))
            }
            Stmt::BlockReturn(expr) => {
                // block_return 文: ブロック式の値として BlockReturn シグナルを返す
                let val = self.eval(expr)?;
                Ok(ExecResult::BlockReturn(val))
            }
            Stmt::LoopYield(expr) => {
                // loop_yield 文: for/while 式のリスト蓄積コンテキスト（BLOCK_YIELDS が Some）にのみ有効。
                // BLOCK_YIELDS が None のとき（for/while 式の外）は実行時エラー。
                let val = self.eval(expr)?;
                let mut in_loop_expr = false;
                BLOCK_YIELDS.with(|y| {
                    if let Some(yields) = y.borrow_mut().as_mut() {
                        yields.push(val);
                        in_loop_expr = true;
                    }
                });
                if !in_loop_expr {
                    return Err("SyntaxError: 'loop_yield' can only be used inside a for/while expression (with ->list[T] annotation)".to_string());
                }
                Ok(ExecResult::Normal)
            }
            Stmt::If { branches, else_body } => {
                // if/elif/else: 各条件を順に評価し、最初に truthy になった本体を実行する
                for (cond, body) in branches {
                    let val = self.eval(cond)?;
                    if self.is_truthy(&val) {
                        return self.exec_scoped_block(body);
                    }
                }
                if let Some(body) = else_body {
                    return self.exec_scoped_block(body);
                }
                Ok(ExecResult::Normal)
            }
            Stmt::Match { subject, arms, .. } => {
                // match 文: subject を評価し、最初にマッチしたアームの本体を実行する
                let subject_val = self.eval(subject)?;
                for arm in arms {
                    let matched = match &arm.pattern {
                        MatchPattern::Case(pattern_expr) => {
                            // `case _:` はワイルドカード — 常にマッチ
                            if matches!(pattern_expr, Expr::Ident(n) if n == "_") {
                                true
                            } else {
                                let pattern_val = self.eval(pattern_expr)?;
                                let result = self.apply_binop(
                                    &crate::ast::BinOp::Eq,
                                    subject_val.clone(),
                                    pattern_val,
                                )?;
                                matches!(result, Value::Bool(true))
                            }
                        }
                        MatchPattern::IsType(type_name) => {
                            self.value_is_type(&subject_val, type_name)
                        }
                    };
                    if matched {
                        return self.exec_scoped_block(&arm.body);
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::While { cond, body } => {
                // while ループ: 条件が falsy になるか break が実行されるまでループする
                LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
                let result = (|| {
                    loop {
                        let val = self.eval(cond)?;
                        if !self.is_truthy(&val) {
                            break;
                        }
                        match self.exec_scoped_block(body)? {
                            ExecResult::Break | ExecResult::BlockReturn(Value::None) => break,
                            ExecResult::Continue | ExecResult::Normal => {}
                            r => return Ok(r),
                        }
                    }
                    Ok(ExecResult::Normal)
                })();
                LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
                result
            }
            Stmt::For { target, iter, body } => {
                let iter_val = self.eval(iter)?;
                // イテレータプロトコル: イテラブル値から Generator を取得する
                let generator = match iter_val {
                    Value::List(items) => {
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items.borrow().clone(), index: 0 })))
                    }
                    Value::Str(s) => {
                        let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: chars, index: 0 })))
                    }
                    Value::Set(items) => {
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items.borrow().clone(), index: 0 })))
                    }
                    Value::Generator(_) => iter_val,
                    Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[])?,
                    Value::PyObject(ref handle) => {
                        let items = super::py_interop::py_collect_iter(handle)?;
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items, index: 0 })))
                    }
                    _ => return Err("TypeError: object is not iterable".to_string()),
                };
                LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
                let result = (|| {
                    loop {
                        match self.eval_method_call(generator.clone(), "next", &[]) {
                            Ok(item) => {
                                self.push_scope();
                                self.declare_var(target.clone(), Var::new(item, true));
                                let result = self.exec_block(body);
                                self.pop_scope();
                                match result? {
                                    ExecResult::Break | ExecResult::BlockReturn(Value::None) => break,
                                    ExecResult::Continue | ExecResult::Normal => {}
                                    r => return Ok(r),
                                }
                            }
                            Err(ref e) if e.starts_with("EndOfIteration") => break,
                            Err(e) => return Err(e),
                        }
                    }
                    Ok(ExecResult::Normal)
                })();
                LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
                result
            }
            Stmt::Block(body) => {
                // block: 文: 値を返さないスコープブロック。
                // - BlockReturn(non-None): 値を捨てて Normal を返す（block: が値を吸収）
                // - BlockReturn(None) (= break): 伝播させる（外側のループが捕捉できるよう）
                // block_yield はスレッドローカル経由で収集される。
                // 値を受け取りたいなら block: を式として使う（Expr::Block）。
                match self.exec_scoped_block(body)? {
                    ExecResult::Normal => Ok(ExecResult::Normal),
                    ExecResult::BlockReturn(v) if !matches!(v, Value::None) => Ok(ExecResult::Normal),
                    r => Ok(r), // BlockReturn(None)=break, Continue, Return, Raise は伝播
                }
            }
            Stmt::FnDef { name, template_params, params, body, decorators, .. } => {
                if !template_params.is_empty() {
                    // テンプレート関数: TemplateFn として格納する（現在はオーバーロード未対応）
                    let tmpl = Rc::new(TemplateFnValue {
                        template_params: template_params.clone(),
                        params: params.clone(),
                        body: body.clone(),
                    });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var::new(Value::TemplateFn(tmpl), false));
                } else {
                    // クロージャキャプチャ: 非グローバルスコープでの定義時のみ実行する
                    let captured_env = if self.scopes.len() > 1 {
                        self.capture_env(body, params)
                    } else {
                        HashMap::new()
                    };
                    let fn_val = Rc::new(FnValue {
                        params: params.clone(),
                        body: body.clone(),
                        is_python: self.in_python_module,
                        captured_env,
                    });

                    if decorators.is_empty() {
                        // デコレータなし: 同名の既存定義があれば OverloadedFn に蓄積する（同スコープレベル内）
                        let existing = self.scopes.last()
                            .and_then(|s| s.get(name.as_str()))
                            .map(|v| v.get_value());
                        let new_value = match existing {
                            Some(Value::Function(prev)) => Value::OverloadedFn(vec![prev, fn_val]),
                            Some(Value::OverloadedFn(mut fns)) => {
                                fns.push(fn_val);
                                Value::OverloadedFn(fns)
                            }
                            _ => Value::Function(fn_val),
                        };
                        self.scopes.last_mut().unwrap()
                            .insert(name.clone(), Var::new(new_value, false));
                    } else {
                        // デコレータあり: 下（末尾）から順に適用する
                        let mut value = Value::Function(fn_val);
                        for dec_expr in decorators.iter().rev() {
                            let dec = self.eval(dec_expr)?;
                            value = self.apply_value_call(dec, value, name)?;
                        }
                        self.scopes.last_mut().unwrap()
                            .insert(name.clone(), Var::new(value, false));
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::Yield(expr) => {
                // yield 文: スレッドローカルの yield コレクタに値を追加する
                let val = self.eval(expr)?;
                GENERATOR_YIELDS.with(|y| {
                    if let Some(yields) = y.borrow_mut().as_mut() {
                        yields.push(val.clone());
                    }
                });
                Ok(ExecResult::Normal)
            }
            Stmt::GenDef { name, template_params, params, body, .. } => {
                // ジェネレータ関数定義: GeneratorFn または TemplateGenFn としてスコープに登録する
                if !template_params.is_empty() {
                    let tmpl = Rc::new(TemplateGenFnValue {
                        template_params: template_params.clone(),
                        params: params.clone(),
                        body: body.clone(),
                    });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var::new(Value::TemplateGenFn(tmpl), false));
                } else {
                    let captured_env = if self.scopes.len() > 1 {
                        self.capture_env(body, params)
                    } else {
                        HashMap::new()
                    };
                    let gen_fn = Rc::new(GeneratorFnValue {
                        params: params.clone(),
                        body: body.clone(),
                        captured_env,
                    });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var::new(Value::GeneratorFn(gen_fn), false));
                }
                Ok(ExecResult::Normal)
            }
            Stmt::TraitDef { name, body, .. } => {
                // trait フィールドのアクセス可能性を収集して trait_field_access に保存する
                let mut trait_access: HashMap<String, Accessibility> = HashMap::new();
                for stmt in body {
                    if let Stmt::Field { name: fname, access, .. } = stmt {
                        if *access != Accessibility::Public {
                            trait_access.insert(fname.clone(), access.clone());
                        }
                    }
                    if let Stmt::FnDef { name: mname, access, .. } = stmt {
                        if *access != Accessibility::Public {
                            trait_access.insert(mname.clone(), access.clone());
                        }
                    }
                }
                if !trait_access.is_empty() {
                    self.trait_field_access.insert(name.clone(), trait_access);
                }
                self.declare_var(name.clone(), Var::new(Value::Trait(name.clone()), false));
                Ok(ExecResult::Normal)
            }
            Stmt::NewTypeDef { name, original } => {
                // new_type 定義: 元の型に基づいて新しい型名のクラスを生成してバインドする
                let orig_val = self.get_val(original)
                    .ok_or_else(|| format!("NameError: type '{original}' is not defined"))?;
                match orig_val {
                    Value::Class(orig_cls) => {
                        // 元の型がクラスの場合: 名前だけ変えて構造的コピーを生成する。
                        // インスタンスは class.name = 新しい名前を持つため、
                        // メソッド内の `Self` が正しく新しい型に解決される。
                        let new_cls = Rc::new(super::ClassValue {
                            name: name.clone(),
                            bases: orig_cls.bases.clone(),
                            methods: orig_cls.methods.clone(),
                            gen_methods: orig_cls.gen_methods.clone(),
                            field_defaults: orig_cls.field_defaults.clone(),
                            class_vars: orig_cls.class_vars.clone(),
                            field_mutability: orig_cls.field_mutability.clone(),
                            field_access: orig_cls.field_access.clone(),
                            method_access: orig_cls.method_access.clone(),
                            static_method_names: orig_cls.static_method_names.clone(),
                            class_method_names: orig_cls.class_method_names.clone(),
                            static_vars: orig_cls.static_vars.clone(),
                        });
                        self.declare_var(name.clone(), Var::new(Value::Class(new_cls), false));
                    }
                    Value::Type(type_name) => {
                        // 元の型がプリミティブの場合: `value` フィールドを持つラッパークラスを自動生成する。
                        // `new_type Meters: int` → `class Meters: mut value: int` と等価
                        let init_body = vec![
                            Stmt::AttrAssign {
                                target: Expr::Attr {
                                    object: Box::new(Expr::Ident("self".to_string())),
                                    attr: "value".to_string(),
                                },
                                value: Expr::Ident("value".to_string()),
                            },
                        ];
                        let init_fn = Rc::new(FnValue {
                            params: vec![
                                crate::ast::Param { name: "self".to_string(), mutable: true, type_ann: None, default: None },
                                crate::ast::Param { name: "value".to_string(), mutable: false, type_ann: Some(type_name.clone()), default: None },
                            ],
                            body: init_body,
                            is_python: false,
                            captured_env: HashMap::new(),
                        });
                        let mut methods = HashMap::new();
                        methods.insert("__init__".to_string(), vec![init_fn]);
                        let new_cls = Rc::new(super::ClassValue {
                            name: name.clone(),
                            bases: vec![],
                            methods,
                            gen_methods: HashMap::new(),
                            field_defaults: vec![],
                            class_vars: HashMap::new(),
                            field_mutability: HashMap::from([("value".to_string(), true)]),
                            field_access: HashMap::new(),
                            method_access: HashMap::new(),
                            static_method_names: std::collections::HashSet::new(),
                            class_method_names: std::collections::HashSet::new(),
                            static_vars: HashMap::new(),
                        });
                        self.declare_var(name.clone(), Var::new(Value::Class(new_cls), false));
                    }
                    _ => {
                        return Err(format!(
                            "TypeError: cannot create new_type from '{original}' — only classes and primitive types are supported"
                        ));
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::EnumDef { name, variants } => {
                // enum_item_Name クラスを生成する（new_type enum_item_Name: int 相当）
                let item_type_name = format!("enum_item_{}", name);
                let init_body = vec![
                    Stmt::AttrAssign {
                        target: Expr::Attr {
                            object: Box::new(Expr::Ident("self".to_string())),
                            attr: "value".to_string(),
                        },
                        value: Expr::Ident("value".to_string()),
                    },
                ];
                let init_fn = Rc::new(FnValue {
                    params: vec![
                        crate::ast::Param { name: "self".to_string(), mutable: true, type_ann: None, default: None },
                        crate::ast::Param { name: "value".to_string(), mutable: false, type_ann: Some("int".to_string()), default: None },
                    ],
                    body: init_body,
                    is_python: false,
                    captured_env: HashMap::new(),
                });
                let mut item_methods = HashMap::new();
                item_methods.insert("__init__".to_string(), vec![init_fn]);
                let item_cls = Rc::new(super::ClassValue {
                    name: item_type_name.clone(),
                    bases: vec![],
                    methods: item_methods,
                    gen_methods: HashMap::new(),
                    field_defaults: vec![],
                    class_vars: HashMap::new(),
                    field_mutability: HashMap::from([("value".to_string(), true)]),
                    field_access: HashMap::new(),
                    method_access: HashMap::new(),
                    static_method_names: std::collections::HashSet::new(),
                    class_method_names: std::collections::HashSet::new(),
                    static_vars: HashMap::new(),
                });
                self.declare_var(item_type_name.clone(), Var::new(Value::Class(item_cls.clone()), false));

                // 各バリアントの値を計算し、enum クラスの const クラス変数として登録する
                let mut class_vars: HashMap<String, Value> = HashMap::new();
                let mut next_value: i64 = 0;
                for (variant_name, value_expr) in variants {
                    let int_val = if let Some(expr) = value_expr {
                        match self.eval(expr)? {
                            Value::Int(n) => n,
                            other => return Err(format!(
                                "TypeError: enum variant '{}' value must be int, got '{}'",
                                variant_name, self.type_name(&other)
                            )),
                        }
                    } else {
                        next_value
                    };
                    next_value = int_val + 1;
                    let inst = self.instantiate_evaled(item_cls.clone(), vec![(None, Value::Int(int_val))])?;
                    class_vars.insert(variant_name.clone(), inst);
                }

                // 列挙型クラスを定数メンバーのみ持つクラスとして登録する
                let enum_cls = Rc::new(super::ClassValue {
                    name: name.clone(),
                    bases: vec![],
                    methods: HashMap::new(),
                    gen_methods: HashMap::new(),
                    field_defaults: vec![],
                    class_vars,
                    field_mutability: HashMap::new(),
                    field_access: HashMap::new(),
                    method_access: HashMap::new(),
                    static_method_names: std::collections::HashSet::new(),
                    class_method_names: std::collections::HashSet::new(),
                    static_vars: HashMap::new(),
                });
                self.declare_var(name.clone(), Var::new(Value::Class(enum_cls), false));
                Ok(ExecResult::Normal)
            }
            Stmt::ClassDef { name, template_params, bases, body, decorators } => {
                if !template_params.is_empty() {
                    // テンプレートクラス: ClassValue をまだ構築せず TemplateClass として格納する
                    let tmpl = Rc::new(TemplateClassValue {
                        name: name.clone(),
                        template_params: template_params.clone(),
                        bases: bases.clone(),
                        body: body.clone(),
                    });
                    self.declare_var(name.clone(), Var::new(Value::TemplateClass(tmpl), false));
                    return Ok(ExecResult::Normal);
                }
                // 通常クラス: クラス本体を走査してメソッド・フィールド・クラス変数を収集する
                let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
                let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
                let mut field_defaults = Vec::new();
                let mut class_vars: HashMap<String, Value> = HashMap::new();
                let mut field_mutability: HashMap<String, bool> = HashMap::new();
                let mut field_access: HashMap<String, Accessibility> = HashMap::new();
                let mut method_access: HashMap<String, Accessibility> = HashMap::new();
                let mut static_method_names: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut class_method_names: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut static_vars: HashMap<String, Rc<RefCell<Value>>> = HashMap::new();
                // 継承トレイトのフィールドアクセス可能性を引き継ぐ
                for base in bases {
                    if let Some(trait_acc) = self.trait_field_access.get(base) {
                        for (fname, acc) in trait_acc {
                            // トレイトフィールドはインスタンス内で "TraitName::field" キーで格納されるため
                            // "TraitName::field" キーでアクセス制御を登録する
                            field_access.insert(format!("{}::{}", base, fname), acc.clone());
                        }
                    }
                }
                for stmt in body {
                    match stmt {
                        Stmt::FnDef { name: mname, params, body: mbody, decorators: mdecs, access: macc, is_static, is_class_method, .. } => {
                            let fn_val = Rc::new(FnValue {
                                params: params.clone(),
                                body: mbody.clone(),
                                is_python: self.in_python_module,
                                captured_env: HashMap::new(),
                            });
                            if *is_static {
                                static_method_names.insert(mname.clone());
                            }
                            if *is_class_method {
                                class_method_names.insert(mname.clone());
                            }
                            if *macc != Accessibility::Public {
                                method_access.insert(mname.clone(), macc.clone());
                            }
                            if mdecs.is_empty() {
                                // デコレータなし: 同名があればオーバーロードとして蓄積する
                                methods.entry(mname.clone()).or_default().push(fn_val);
                            } else {
                                // デコレータあり: 下から順に適用し、関数値として蓄積する
                                let mut value = Value::Function(fn_val);
                                for dec_expr in mdecs.iter().rev() {
                                    let dec = self.eval(dec_expr)?;
                                    value = self.apply_value_call(dec, value, mname)?;
                                }
                                match value {
                                    Value::Function(f) => methods.entry(mname.clone()).or_default().push(f),
                                    other => return Err(format!(
                                        "TypeError: method decorator on '{}' must return a function, got '{}'",
                                        mname, self.type_name(&other)
                                    )),
                                }
                            }
                        }
                        Stmt::GenDef { name: mname, params, body: mbody, access: macc, .. } => {
                            // ジェネレータメソッド定義: gen_methods に登録する
                            if *macc != Accessibility::Public {
                                method_access.insert(mname.clone(), macc.clone());
                            }
                            gen_methods.insert(mname.clone(), Rc::new(GeneratorFnValue {
                                params: params.clone(),
                                body: mbody.clone(),
                                captured_env: HashMap::new(),
                            }));
                        }
                        Stmt::Field { name: fname, kind: FieldKind::Const, default: Some(init), access: facc, .. } => {
                            // const クラス変数: 初期値を評価して class_vars に登録する
                            if *facc != Accessibility::Public {
                                field_access.insert(fname.clone(), facc.clone());
                            }
                            let val = self.eval(init)?;
                            class_vars.insert(fname.clone(), val);
                        }
                        Stmt::Field { name: fname, kind: FieldKind::StaticMut, default, access: facc, .. } => {
                            // static mut クラス静的変数: 全インスタンスで共有される可変セルを作成する
                            if *facc != Accessibility::Public {
                                field_access.insert(fname.clone(), facc.clone());
                            }
                            let val = if let Some(init) = default {
                                self.eval(init)?
                            } else {
                                Value::None
                            };
                            static_vars.insert(fname.clone(), Rc::new(RefCell::new(val)));
                        }
                        Stmt::Field { name: fname, kind, default, access: facc, .. } => {
                            // mut / let インスタンスフィールド: 可変フラグと初期値を記録する
                            if *facc != Accessibility::Public {
                                field_access.insert(fname.clone(), facc.clone());
                            }
                            let mutable = *kind == FieldKind::Mut;
                            field_mutability.insert(fname.clone(), mutable);
                            if let Some(init) = default {
                                let val = self.eval(init)?;
                                field_defaults.push((fname.clone(), val, mutable));
                            }
                        }
                        _ => {}
                    }
                }
                let cls = Rc::new(super::ClassValue {
                    name: name.clone(),
                    bases: bases.clone(),
                    methods,
                    gen_methods,
                    field_defaults,
                    class_vars,
                    field_mutability,
                    field_access,
                    method_access,
                    static_method_names,
                    class_method_names,
                    static_vars,
                });
                if decorators.is_empty() {
                    self.declare_var(name.clone(), Var::new(Value::Class(cls), false));
                } else {
                    // デコレータあり: 下（末尾）から順に適用する
                    let mut value = Value::Class(cls);
                    for dec_expr in decorators.iter().rev() {
                        let dec = self.eval(dec_expr)?;
                        value = self.apply_value_call(dec, value, name)?;
                    }
                    self.declare_var(name.clone(), Var::new(value, false));
                }
                Ok(ExecResult::Normal)
            }
            Stmt::Freeze(name, span) => {
                // freeze 文: 変数を不変化する。インスタンスの場合は freeze_instance も呼ぶ。
                let var = self.get_var(name)
                    .ok_or_else(|| format!("{span}: NameError: '{name}' is not defined"))?;
                if !var.mutable {
                    return Err(format!(
                        "{span}: TypeError: cannot freeze immutable variable '{name}'"
                    ));
                }
                // クロージャにキャプチャされた可変変数は freeze できない
                if var.mutable_cell.is_some() {
                    return Err(format!(
                        "{span}: TypeError: cannot freeze '{name}' because it is captured by a closure"
                    ));
                }
                let val = var.get_value();

                if let Value::Instance(ref inst_rc) = val {
                    let class = inst_rc.borrow().class.clone();
                    // `__freeze__` メソッドが定義されている場合は凍結前に呼び出す
                    if let Some(overloads) = self.lookup_method_in_class(&class, "__freeze__") {
                        if overloads.len() == 1 {
                            self.exec_fn(overloads[0].clone(), &[], Some(val.clone()), "__freeze__")?;
                        } else {
                            self.dispatch_overload(overloads, &[], Some(val.clone()))?;
                        }
                    }
                    Self::freeze_instance(inst_rc);
                }

                self.make_var_immutable(name);
                Ok(ExecResult::Normal)
            }

            Stmt::Raise { exc, span } => {
                // 裸の `raise`: アクティブな例外を再 raise する
                if exc.is_none() {
                    match &self.current_exception {
                        Some(err) => {
                            let err = err.clone();
                            return Ok(ExecResult::Raise(err));
                        }
                        None => return Err("RuntimeError: no active exception to re-raise".to_string()),
                    }
                }

                let exc_val = self.eval(exc.as_ref().unwrap())?;

                // 例外インスタンスに file / line / col / code_context を直接書き込む
                if let Value::Instance(ref inst_rc) = exc_val {
                    let context = self.get_context_lines(&span.file, span.line, 5);
                    let mut inst = inst_rc.borrow_mut();
                    // 通常属性アクセス（`e.file` など）で参照できるように直接フィールドに設定する
                    inst.fields.insert("file".to_string(),         (Value::Str(span.file.to_string()), true));
                    inst.fields.insert("line".to_string(),         (Value::Int(span.line as i64),       true));
                    inst.fields.insert("col".to_string(),          (Value::Int(span.col as i64),        true));
                    inst.fields.insert("code_context".to_string(), (Value::Str(context.clone()),        true));
                    // トレイトフィールドアクセス（`Error::file` など）のために名前空間付きキーでも登録する
                    inst.fields.insert("Error::file".to_string(),         (Value::Str(span.file.to_string()), true));
                    inst.fields.insert("Error::line".to_string(),         (Value::Int(span.line as i64),       true));
                    inst.fields.insert("Error::col".to_string(),          (Value::Int(span.col as i64),        true));
                    inst.fields.insert("Error::code_context".to_string(), (Value::Str(context),                true));
                }

                // raise 地点のスタックフレームを生成して ExecResult::Raise に包む
                let fn_name = self.call_stack.last().cloned().unwrap_or_else(|| "<module>".to_string());
                let frame = StackFrame {
                    file: span.file.to_string(),
                    line: span.line,
                    col: span.col,
                    fn_name,
                    context: self.get_context_lines(&span.file, span.line, 5),
                };
                Ok(ExecResult::Raise(RaisedError { exception: exc_val, frames: vec![frame] }))
            }

            Stmt::Try { body, handlers, finally_body } => {
                // try ブロックを実行して例外の発生を検査する
                let body_result = self.exec_scoped_block(body);

                // ボディが Raise シグナルを返したか、例外センチネルを返したかを判定する
                let raise_opt: Option<RaisedError> = match &body_result {
                    Ok(ExecResult::Raise(r)) => Some(r.clone()),
                    Err(e) if e.as_str() == RAISE_SENTINEL => self.current_exception.clone(),
                    _ => None,
                };

                let mut final_result: Result<ExecResult, String> = body_result;

                if let Some(raised) = raise_opt {
                    let mut handled = false;
                    for handler in handlers {
                        let matches = match &handler.exc_type {
                            None => true, // 裸の `except:` はすべての例外を捕捉する
                            Some(type_name) => {
                                if let Value::Instance(ref inst_rc) = raised.exception {
                                    Self::exc_matches(&inst_rc.borrow().class, type_name)
                                } else {
                                    false
                                }
                            }
                        };
                        if matches {
                            // except ブロック内で current_exception を設定し（裸の raise 用）、
                            // ブロック終了後に元の値を復元する
                            let prev_exc = self.current_exception.clone();
                            self.current_exception = Some(raised.clone());

                            self.push_scope();
                            if let Some(alias) = &handler.name {
                                // `except ValueError as e:` の `e` をスコープに束縛する
                                let exc_val = raised.exception.clone();
                                self.declare_var(alias.clone(), Var::new(exc_val, false));
                            }
                            let handler_result = self.exec_block(&handler.body);
                            self.pop_scope();

                            self.current_exception = prev_exc;
                            final_result = handler_result;
                            handled = true;
                            break;
                        }
                    }
                    if !handled {
                        // どの handler にもマッチしなかった場合:
                        // final_result はそのまま body_result を維持し、元の伝播パスを保持する
                        // （直接 raise は Ok(ExecResult::Raise)、関数経由は Err(sentinel)）
                    }
                }

                // finally ブロックを結果に関わらず実行する
                if let Some(finally) = finally_body {
                    let finally_result = self.exec_scoped_block(finally);
                    // finally 自体が raise / return した場合はそちらが優先される
                    match finally_result {
                        Ok(ExecResult::Normal) => {}
                        Ok(signal) => return Ok(signal),
                        Err(e) => return Err(e),
                    }
                }

                final_result
            }

            // ------------------------------------------------------------------
            // import[lang] module as alias
            // ------------------------------------------------------------------
            Stmt::Import { lang, module, alias, body } => {
                let ns = self.exec_module(lang, module, body)?;
                let bind_name = alias.clone()
                    .unwrap_or_else(|| module.last().unwrap().clone());
                self.declare_var(bind_name, Var::new(Value::Namespace(ns), false));
                Ok(ExecResult::Normal)
            }

            // ------------------------------------------------------------------
            // from module import[lang] Name1, Name2 as N2
            // ------------------------------------------------------------------
            Stmt::FromImport { lang, module, names, body } => {
                let ns = self.exec_module(lang, module, body)?;
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let val = ns.members.get(orig_name.as_str())
                        .cloned()
                        .ok_or_else(|| format!(
                            "ImportError: cannot import name '{}' from '{}'",
                            orig_name, module.join(".")
                        ))?;
                    self.declare_var(bind_name, Var::new(val, false));
                }
                Ok(ExecResult::Normal)
            }

            // ------------------------------------------------------------------
            // target <- async [->Type]: body
            // ------------------------------------------------------------------
            Stmt::AsyncAssign { target, stmts, .. } => {
                // Resolve the AsyncManager value
                let mgr_val = self.get_var(target)
                    .map(|v| v.get_value())
                    .ok_or_else(|| format!("NameError: '{}' is not defined", target))?;

                let mgr_rc = match mgr_val {
                    Value::AsyncManager(rc) => rc,
                    other => return Err(format!(
                        "TypeError: '<-' operator requires an AsyncManager, got '{}'",
                        self.type_name(&other)
                    )),
                };

                // Deep-clone the current scope environment for the thread
                let env = super::async_mgr::capture_env(self);

                // Submit the task
                mgr_rc.borrow_mut().add_task(stmts.clone(), env);

                Ok(ExecResult::Normal)
            }
        }
    }

    /// モジュールの body を孤立スコープで実行し、`Value::Namespace` を返す。
    /// キャッシュを使用し、循環 import はエラーにする。
    fn exec_module(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
    ) -> Result<Rc<NamespaceData>, String> {
        // キャッシュキーにはモジュール名を文字列結合で代用（パース時に解決済み）
        let cache_key = (lang.to_string(), PathBuf::from(module.join("/")));

        match self.module_cache.get(&cache_key).cloned() {
            Some(ModuleState::Loading) => {
                return Err(format!(
                    "RuntimeError: circular import detected: '{}'",
                    module.join(".")
                ));
            }
            Some(ModuleState::Loaded(ns)) => return Ok(ns),
            None => {}
        }

        // Loading マーカーをセット
        self.module_cache.insert(cache_key.clone(), ModuleState::Loading);

        // py-int モジュール: body は型シグネチャ用（型検査済み）。実行時は PyO3 経由でロードする
        if lang == "py-int" {
            let search_dirs = self.python_search_dirs.clone();
            let ns = super::py_interop::load_py_int_module(module, &search_dirs)
                .map_err(|e| e)?;
            self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
            return Ok(ns);
        }

        // tl モジュール: .tlc v1 に埋め込まれたネイティブ DLL がキャッシュにあれば優先する
        // import[tl] (lang=="tl") は明示的にソースを要求しているのでスキップする
        if lang == "tl-auto" || lang == "tlc" {
            let module_name = module.join(".");
            if let Some((_exports, dll_bytes)) = crate::partial_compiler::take_native_bytes(&module_name) {
                let ext = crate::partial_compiler::native_lib_ext();
                let stem = module.last().cloned().unwrap_or_default();
                // Write DLL bytes to a deterministic temp path and load from there.
                let tmp_path = std::env::temp_dir().join(format!("{stem}_tl.{ext}"));
                match std::fs::write(&tmp_path, &dll_bytes) {
                    Ok(()) => {
                        match self.try_load_native_module(module, body, &tmp_path) {
                            Ok(ns) => {
                                self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
                                return Ok(ns);
                            }
                            Err(e) => {
                                eprintln!("NativeLoad: failed: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("NativeLoad: cannot write temp DLL: {e}");
                    }
                }
            }
        }

        // body を孤立スコープで実行してトップレベル名を収集
        let prev_in_python = self.in_python_module;
        if lang == "py" { self.in_python_module = true; }
        self.push_scope();
        for stmt in body {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                ExecResult::Raise(_) => {
                    self.pop_scope();
                    return Err(format!(
                        "RuntimeError: exception during module initialization: {}",
                        module.join(".")
                    ));
                }
                _ => {}
            }
        }
        let members: HashMap<String, Value> = self
            .scopes
            .last()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect();
        self.pop_scope();
        self.in_python_module = prev_in_python;

        // モジュールメンバをグローバルスコープに登録する。
        // Python モジュールのメソッドが同モジュール内の他の関数を呼び出せるようにするための措置。
        // 既存のグローバル名は上書きしない（or_insert を使用）。
        for (name, value) in &members {
            self.scopes[0].entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        let ns = Rc::new(NamespaceData { name: module.join("."), members });
        self.module_cache.insert(cache_key, ModuleState::Loaded(ns.clone()));
        Ok(ns)
    }

    /// ネイティブ共有ライブラリをロードして、そのモジュールの `Namespace` を構築する。
    ///
    /// - 見つかった DLL シンボル (`fn_name_tl`) に対して `Value::NativeFunction` を作成する。
    /// - DLL に存在しない関数は tree-walk 実行のためそのまま残す（body を通常実行して収集）。
    fn try_load_native_module(
        &mut self,
        module: &[String],
        body: &[Stmt],
        lib_path: &std::path::Path,
    ) -> Result<Rc<NamespaceData>, String> {
        // Load the shared library.
        let lib = unsafe { libloading::Library::new(lib_path) }
            .map_err(|e| format!("libloading: {e}"))?;

        let lib_path_buf = lib_path.to_path_buf();

        // First, execute the body to collect all members via tree-walk.
        // (This also registers non-native functions, classes, etc.)
        self.push_scope();
        for stmt in body {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                ExecResult::Raise(_raised) => {
                    self.pop_scope();
                    return Err(format!(
                        "RuntimeError: exception during native module init: {}",
                        module.join(".")
                    ));
                }
                _ => {}
            }
        }
        let mut members: HashMap<String, Value> = self
            .scopes
            .last()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.get_value()))
            .collect();
        self.pop_scope();

        // Override tree-walk versions with native versions where a symbol exists.
        for stmt in body {
            if let Stmt::FnDef { name, params, .. } = stmt {
                let symbol_name = format!("{name}_tl\0");
                let has_symbol = unsafe {
                    lib.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(
                        symbol_name.as_bytes()
                    ).is_ok()
                };
                if has_symbol {
                    let fn_ref = Rc::new(NativeFnRef {
                        lib_path: lib_path_buf.clone(),
                        fn_name: name.clone(),
                        n_params: params.len(),
                        param_mutabilities: params.iter().map(|p| p.mutable).collect(),
                    });
                    members.insert(name.clone(), Value::NativeFunction(fn_ref));
                }
            }
        }

        // Register globally so intra-module calls among non-native functions work.
        for (name, value) in &members {
            self.scopes[0]
                .entry(name.clone())
                .or_insert_with(|| Var::new(value.clone(), false));
        }

        // Call tl_init so the DLL can store the callbacks pointer.
        {
            let cb_ptr = super::native_api::get_callbacks();
            let symbol_name = b"tl_init\0";
            let init_result = unsafe {
                lib.get::<unsafe extern "C" fn(*const super::native_api::TlCallbacks)>(symbol_name)
            };
            if let Ok(tl_init) = init_result {
                unsafe { tl_init(cb_ptr) };
            }
        }

        // Store the loaded library so it stays alive for the interpreter's lifetime.
        self.native_libs.insert(lib_path_buf, NativeLibWrapper(lib));

        let ns = Rc::new(NamespaceData {
            name: module.join("."),
            members,
        });
        Ok(ns)
    }

    /// 文のリストを順に実行する。通常終了以外のシグナル（Break / Return / Raise 等）が発生したら即返す。
    ///
    /// - `stmts`: 実行する文のスライス
    ///
    /// 戻り値: 最初に発生した非 Normal な `ExecResult`、またはすべて Normal なら `ExecResult::Normal`
    pub(super) fn exec_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        for stmt in stmts {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                signal => return Ok(signal), // Break / Continue / Return / Raise などを即上位に返す
            }
        }
        Ok(ExecResult::Normal)
    }

    /// 新しいスコープを積んでから文のリストを実行し、完了後にスコープを取り除く。
    /// if/while/for の本体など、スコープを分離したいブロック実行に使用する。
    ///
    /// - `stmts`: 実行する文のスライス
    ///
    /// 戻り値: `exec_block` と同じ
    pub(super) fn exec_scoped_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        self.push_scope();
        let result = self.exec_block(stmts);
        self.pop_scope();
        result
    }

    // ---------------------------------------------------------------------------
    // クロージャキャプチャ
    // ---------------------------------------------------------------------------

    /// 関数本体のフリー変数を分析して、現在の非グローバルスコープからキャプチャ環境を構築する。
    ///
    /// - 不変変数: ディープコピーして `CapturedVar::Immutable` として格納する
    /// - 可変変数: 共有セルを作成（既存のセルを再利用）して `CapturedVar::Mutable` として格納する。
    ///   可変変数のスコープエントリも同じセルを参照するよう更新される。
    pub(super) fn capture_env(
        &mut self,
        body: &[Stmt],
        params: &[Param],
    ) -> HashMap<String, CapturedVar> {
        // 関数自身のスコープで宣言される名前（パラメータ + 本体内の宣言）
        let mut own_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        collect_declared_names(body, &mut own_names);

        // 本体内で参照されるすべての識別子名
        let mut referenced: HashSet<String> = HashSet::new();
        collect_referenced_names(body, &mut referenced);

        // フリー変数 = 参照名 - 自分のスコープで宣言される名前
        let free_vars: Vec<String> = referenced
            .into_iter()
            .filter(|n| !own_names.contains(n))
            .collect();

        let mut captured: HashMap<String, CapturedVar> = HashMap::new();
        let n_scopes = self.scopes.len();

        for name in &free_vars {
            // スコープ[1..] を内側から外側へ向けて検索（グローバルスコープはキャプチャしない）
            for scope_idx in (1..n_scopes).rev() {
                let found = self.scopes[scope_idx].get(name.as_str()).map(|var| {
                    (var.mutable, var.mutable_cell.clone(), var.get_value())
                });

                if let Some((is_mutable, existing_cell, current_value)) = found {
                    if is_mutable {
                        // 可変変数: 既存のセルを再利用するか、新しいセルを作成する
                        let cell = if let Some(cell) = existing_cell {
                            cell
                        } else {
                            let cell = Rc::new(RefCell::new(current_value));
                            // スコープのエントリも同じセルを参照するよう更新する
                            if let Some(var) = self.scopes[scope_idx].get_mut(name.as_str()) {
                                var.mutable_cell = Some(cell.clone());
                            }
                            cell
                        };
                        captured.insert(name.clone(), CapturedVar::Mutable(cell));
                    } else {
                        // 不変変数: ディープコピーして保持する
                        captured.insert(name.clone(), CapturedVar::Immutable(
                            Self::deep_copy_value(current_value),
                        ));
                    }
                    break; // このスコープで見つかったので外側は検索しない
                }
            }
        }

        captured
    }

    /// 評価済みの値 `callee` を単一の評価済み引数 `arg` で呼び出す（デコレータ適用用）。
    ///
    /// - `Value::Function` / `Value::OverloadedFn` → 直接呼び出す
    /// - `Value::Class` → `instantiate_evaled` でインスタンス化する
    /// - `Value::Instance` → `__call__` メソッドに委譲する
    pub(super) fn apply_value_call(&mut self, callee: Value, arg: Value, label: &str) -> Result<Value, String> {
        let evaled = vec![(None, arg)];
        match callee {
            Value::Function(fn_val) => self.exec_fn_evaled(fn_val, &evaled, None, label),
            Value::OverloadedFn(candidates) => self.dispatch_overload_evaled(candidates, evaled, None, label),
            Value::Class(cls) => self.instantiate_evaled(cls, evaled),
            Value::Instance(ref inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let overloads = self.lookup_method_in_class(&class, "__call__")
                    .ok_or_else(|| format!(
                        "TypeError: '{}' object is not callable (no __call__ method)", class.name
                    ))?;
                if overloads.len() == 1 {
                    self.exec_fn_evaled(overloads[0].clone(), &evaled, Some(callee), "__call__")
                } else {
                    self.dispatch_overload_evaled(overloads, evaled, Some(callee), "__call__")
                }
            }
            other => Err(format!("TypeError: '{}' object is not callable as decorator", self.type_name(&other))),
        }
    }
}

// ---------------------------------------------------------------------------
// フリー変数分析ヘルパー（モジュールプライベート）
// ---------------------------------------------------------------------------

/// 文リスト中で宣言されるすべての名前を `out` に追加する（保守的・過大評価）。
/// 内側スコープ（if/while/for/block 本体）も再帰的に処理する。
fn collect_declared_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let(name, _) | Stmt::Const(name, _) | Stmt::Mut(name, _)
            | Stmt::Static(name, _, _) => {
                out.insert(name.clone());
            }
            Stmt::FnDef { name, .. } | Stmt::GenDef { name, .. }
            | Stmt::ClassDef { name, .. } | Stmt::TraitDef { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::For { target, body, .. } => {
                out.insert(target.clone());
                collect_declared_names(body, out);
            }
            Stmt::If { branches, else_body } => {
                for (_, body) in branches {
                    collect_declared_names(body, out);
                }
                if let Some(body) = else_body {
                    collect_declared_names(body, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Block(body) => {
                collect_declared_names(body, out);
            }
            Stmt::Try { body, handlers, finally_body } => {
                collect_declared_names(body, out);
                for h in handlers {
                    if let Some(alias) = &h.name {
                        out.insert(alias.clone());
                    }
                    collect_declared_names(&h.body, out);
                }
                if let Some(body) = finally_body {
                    collect_declared_names(body, out);
                }
            }
            _ => {}
        }
    }
}

/// 文リスト中で参照されるすべての識別子名を `out` に追加する。
/// 内側関数の本体も再帰的に処理する（ネストしたクロージャのため）。
fn collect_referenced_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        collect_referenced_names_stmt(stmt, out);
    }
}

fn collect_referenced_names_stmt(stmt: &Stmt, out: &mut HashSet<String>) {
    match stmt {
        Stmt::Expr(e) => collect_refs_expr(e, out),
        Stmt::Let(_, e) | Stmt::Const(_, e) | Stmt::Mut(_, e) | Stmt::Static(_, e, _) => {
            collect_refs_expr(e, out);
        }
        Stmt::Assign { name, value, .. } => {
            // 代入先の変数も「参照」として扱う（外側スコープへの書き込みのため）
            out.insert(name.clone());
            collect_refs_expr(value, out);
        }
        Stmt::CompoundAssign { name, value, .. } => {
            // 複合代入は読み取り + 書き込みの両方
            out.insert(name.clone());
            collect_refs_expr(value, out);
        }
        Stmt::AttrAssign { target, value } | Stmt::AttrCompoundAssign { target, value, .. } => {
            collect_refs_expr(target, out);
            collect_refs_expr(value, out);
        }
        Stmt::Return(Some(e)) | Stmt::BlockReturn(e) | Stmt::LoopYield(e) | Stmt::Yield(e) => {
            collect_refs_expr(e, out);
        }
        Stmt::Raise { exc: Some(e), .. } => collect_refs_expr(e, out),
        Stmt::If { branches, else_body } => {
            for (cond, body) in branches {
                collect_refs_expr(cond, out);
                collect_referenced_names(body, out);
            }
            if let Some(body) = else_body {
                collect_referenced_names(body, out);
            }
        }
        Stmt::While { cond, body } => {
            collect_refs_expr(cond, out);
            collect_referenced_names(body, out);
        }
        Stmt::For { iter, body, .. } => {
            collect_refs_expr(iter, out);
            collect_referenced_names(body, out);
        }
        Stmt::Block(body) => collect_referenced_names(body, out),
        Stmt::FnDef { body, .. } | Stmt::GenDef { body, .. } => {
            // 内側関数の本体も参照する（ネストしたクロージャがさらに外側を参照するため）
            collect_referenced_names(body, out);
        }
        Stmt::ClassDef { body, .. } | Stmt::TraitDef { body, .. } => {
            collect_referenced_names(body, out);
        }
        Stmt::Try { body, handlers, finally_body } => {
            collect_referenced_names(body, out);
            for h in handlers {
                collect_referenced_names(&h.body, out);
            }
            if let Some(body) = finally_body {
                collect_referenced_names(body, out);
            }
        }
        Stmt::Freeze(name, _) => { out.insert(name.clone()); }
        _ => {}
    }
}

fn collect_refs_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => { out.insert(name.clone()); }
        Expr::BinOp { left, right, .. } => {
            collect_refs_expr(left, out);
            collect_refs_expr(right, out);
        }
        Expr::UnaryOp { operand, .. } => collect_refs_expr(operand, out),
        Expr::Call { func, args } => {
            collect_refs_expr(func, out);
            for arg in args { collect_refs_expr(arg.expr(), out); }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => {
            collect_refs_expr(object, out);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items { collect_refs_expr(item, out); }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                collect_refs_expr(k, out);
                collect_refs_expr(v, out);
            }
        }
        Expr::Subscript { object, index } => {
            collect_refs_expr(object, out);
            collect_refs_expr(index, out);
        }
        Expr::Slice { begin, end, step } => {
            if let Some(e) = begin { collect_refs_expr(e, out); }
            if let Some(e) = end   { collect_refs_expr(e, out); }
            if let Some(e) = step  { collect_refs_expr(e, out); }
        }
        Expr::TemplateInstantiate { base, .. } => collect_refs_expr(base, out),
        Expr::IsType { expr, .. } => collect_refs_expr(expr, out),
        _ => {} // Int, Float, Str, Bool, None
    }
}
