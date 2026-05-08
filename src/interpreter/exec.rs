// exec.rs — 文の実行 (exec / exec_block / exec_scoped_block)
//
// `Interpreter::exec` が文（`Stmt`）を再帰的にツリーウォークして `ExecResult` を返す。
// 変数宣言・代入・制御構造・関数/クラス定義・例外処理など、すべての文の実行を担当する。

use std::cell::RefCell;
use std::rc::Rc;
use std::collections::HashMap;

use crate::ast::{Expr, FieldKind, Stmt};

use super::{
    Interpreter, Value, Var, ExecResult,
    FnValue, TemplateFnValue, GeneratorFnValue, TemplateGenFnValue, TemplateClassValue,
    GeneratorState,
    RaisedError, StackFrame,
    RAISE_SENTINEL, GENERATOR_YIELDS,
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
    /// - `Return` / `Break` / `Continue` / `BlockReturn` / `BlockYield`: 制御フロー信号を返す
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
                // 不変変数宣言: インスタンス値の場合は freeze する
                let value = self.eval(expr)?;
                if let Value::Instance(ref inst_rc) = value {
                    Self::freeze_instance(inst_rc);
                }
                self.declare_var(name.clone(), Var { value, mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::Const(name, expr) => {
                // 定数宣言: 常に不変として登録する
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var { value, mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, expr) => {
                // 可変変数宣言: mutable フラグを true にして登録する
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var { value, mutable: true });
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
                    Some(v) => v.value.clone(),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                };
                let value = self.apply_binop(op, lhs, rhs)?;
                self.assign_var(name, value)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Field { .. } => Ok(ExecResult::Normal), // クラス本体内でのみ有効（exec では何もしない）
            Stmt::Break => Ok(ExecResult::Break),
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
            Stmt::BlockYield(expr) => {
                // block_yield 文: block_return と同様に BlockReturn シグナルとして返す
                let val = self.eval(expr)?;
                Ok(ExecResult::BlockReturn(val))
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
            Stmt::While { cond, body } => {
                // while ループ: 条件が falsy になるか break が実行されるまでループする
                loop {
                    let val = self.eval(cond)?;
                    if !self.is_truthy(&val) {
                        break;
                    }
                    match self.exec_scoped_block(body)? {
                        ExecResult::Break => break,
                        ExecResult::Continue | ExecResult::Normal => {}
                        r => return Ok(r), // Return / Raise などはそのまま上位に伝播させる
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::For { target, iter, body } => {
                let iter_val = self.eval(iter)?;
                // イテレータプロトコル: イテラブル値から Generator を取得する
                let generator = match iter_val {
                    Value::List(items) => {
                        // リストをそのままジェネレータにラップする
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items, index: 0 })))
                    }
                    Value::Str(s) => {
                        // 文字列を1文字ずつに展開してジェネレータにラップする
                        let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                        Value::Generator(Rc::new(RefCell::new(GeneratorState { values: chars, index: 0 })))
                    }
                    Value::Generator(_) => iter_val,
                    Value::Instance(_) => {
                        // インスタンスは `__iter__()` を呼び出してジェネレータを取得する
                        self.eval_method_call(iter_val, "__iter__", &[])?
                    }
                    _ => return Err("TypeError: object is not iterable".to_string()),
                };
                // ジェネレータから `next()` を繰り返し呼び出してループ変数に束縛する
                loop {
                    match self.eval_method_call(generator.clone(), "next", &[]) {
                        Ok(item) => {
                            self.push_scope();
                            self.declare_var(target.clone(), Var { value: item, mutable: true });
                            let result = self.exec_block(body);
                            self.pop_scope();
                            match result? {
                                ExecResult::Break => break,
                                ExecResult::Continue | ExecResult::Normal => {}
                                r => return Ok(r),
                            }
                        }
                        // EndOfIteration: ジェネレータ枯渇でループ終了（エラーは伝播させない）
                        Err(ref e) if e.starts_with("EndOfIteration") => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::Block(body) => {
                // block 式: BlockReturn は Normal に変換する（ブロック外に値を返さない）
                match self.exec_scoped_block(body)? {
                    ExecResult::BlockReturn(_) | ExecResult::Normal => Ok(ExecResult::Normal),
                    r => Ok(r),
                }
            }
            Stmt::FnDef { name, template_params, params, body, .. } => {
                if !template_params.is_empty() {
                    // テンプレート関数: TemplateFn として格納する（現在はオーバーロード未対応）
                    let tmpl = Rc::new(TemplateFnValue {
                        template_params: template_params.clone(),
                        params: params.clone(),
                        body: body.clone(),
                    });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var { value: Value::TemplateFn(tmpl), mutable: false });
                } else {
                    let fn_val = Rc::new(FnValue { params: params.clone(), body: body.clone() });

                    // 同名の既存定義があれば OverloadedFn に蓄積する（同スコープレベル内）
                    let existing = self.scopes.last()
                        .and_then(|s| s.get(name.as_str()))
                        .map(|v| v.value.clone());
                    let new_value = match existing {
                        Some(Value::Function(prev)) => Value::OverloadedFn(vec![prev, fn_val]),
                        Some(Value::OverloadedFn(mut fns)) => {
                            fns.push(fn_val);
                            Value::OverloadedFn(fns)
                        }
                        _ => Value::Function(fn_val),
                    };
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var { value: new_value, mutable: false });
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
                        .insert(name.clone(), Var { value: Value::TemplateGenFn(tmpl), mutable: false });
                } else {
                    let gen_fn = Rc::new(GeneratorFnValue { params: params.clone(), body: body.clone() });
                    self.scopes.last_mut().unwrap()
                        .insert(name.clone(), Var { value: Value::GeneratorFn(gen_fn), mutable: false });
                }
                Ok(ExecResult::Normal)
            }
            Stmt::TraitDef { name, .. } => {
                // trait 定義: Value::Trait として不変バインドする
                self.declare_var(name.clone(), Var { value: Value::Trait(name.clone()), mutable: false });
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
                        });
                        self.declare_var(name.clone(), Var { value: Value::Class(new_cls), mutable: false });
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
                                crate::ast::Param { name: "self".to_string(), mutable: true, type_ann: None },
                                crate::ast::Param { name: "value".to_string(), mutable: false, type_ann: Some(type_name.clone()) },
                            ],
                            body: init_body,
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
                        });
                        self.declare_var(name.clone(), Var { value: Value::Class(new_cls), mutable: false });
                    }
                    _ => {
                        return Err(format!(
                            "TypeError: cannot create new_type from '{original}' — only classes and primitive types are supported"
                        ));
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::ClassDef { name, template_params, bases, body } => {
                if !template_params.is_empty() {
                    // テンプレートクラス: ClassValue をまだ構築せず TemplateClass として格納する
                    let tmpl = Rc::new(TemplateClassValue {
                        name: name.clone(),
                        template_params: template_params.clone(),
                        bases: bases.clone(),
                        body: body.clone(),
                    });
                    self.declare_var(name.clone(), Var { value: Value::TemplateClass(tmpl), mutable: false });
                    return Ok(ExecResult::Normal);
                }
                // 通常クラス: クラス本体を走査してメソッド・フィールド・クラス変数を収集する
                let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
                let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
                let mut field_defaults = Vec::new();
                let mut class_vars: HashMap<String, Value> = HashMap::new();
                let mut field_mutability: HashMap<String, bool> = HashMap::new();
                for stmt in body {
                    match stmt {
                        Stmt::FnDef { name: mname, params, body: mbody, .. } => {
                            // 通常メソッド定義: 同名があればオーバーロードとして蓄積する
                            methods.entry(mname.clone()).or_default().push(Rc::new(FnValue {
                                params: params.clone(),
                                body: mbody.clone(),
                            }));
                        }
                        Stmt::GenDef { name: mname, params, body: mbody, .. } => {
                            // ジェネレータメソッド定義: gen_methods に登録する
                            gen_methods.insert(mname.clone(), Rc::new(GeneratorFnValue {
                                params: params.clone(),
                                body: mbody.clone(),
                            }));
                        }
                        Stmt::Field { name: fname, kind: FieldKind::Const, default: Some(init), .. } => {
                            // const クラス変数: 初期値を評価して class_vars に登録する
                            let val = self.eval(init)?;
                            class_vars.insert(fname.clone(), val);
                        }
                        Stmt::Field { name: fname, kind, default, .. } => {
                            // mut / let インスタンスフィールド: 可変フラグと初期値を記録する
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
                });
                self.declare_var(name.clone(), Var { value: Value::Class(cls), mutable: false });
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
                let val = var.value.clone();

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
                                self.declare_var(alias.clone(), Var { value: exc_val, mutable: false });
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
        }
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
}
