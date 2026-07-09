// exec/definitions.rs — 定義文の実行: 関数/ジェネレータ定義、トレイト/プロトコル/new_type/enum/クラス定義。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::collections::{HashMap, HashSet}, std::path::PathBuf,
    std::rc::Rc, std::sync::Arc,
    crate::ast::{
        Accessibility, BinOp, ExceptHandler, Expr, FieldKind, MatchArm, MatchPattern, Param,
        Stmt, TemplateParam, TupleTarget,
    },
    crate::token::Span,
    crate::interpreter::{
        debugger::DbgMode, CapturedVar, ExecResult, FnValue, GeneratorFnValue, GeneratorState,
        Interpreter, ModuleState, NamespaceData, NativeFnRef, NativeLibWrapper, RaisedError,
        StackFrame, TemplateClassValue, TemplateFnValue, TemplateGenFnValue, Value, Var,
        BLOCK_RETURN_EXPECTED_TYPE, BLOCK_YIELDS, BREAK_SENTINEL, GENERATOR_YIELDS, LOOP_DEPTH,
        RAISE_SENTINEL,
    },
};
#[allow(unused_imports)]
use super::*;

impl Interpreter {
    /// `fn` 定義を実行して関数値をスコープに登録する。テンプレート関数はテンプレート値として格納する。
    pub(crate) fn exec_fn_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        params: &[Param],
        body: &[Stmt],
        decorators: &[Expr],
        return_type: Option<&str>,
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateFnValue {
                name: name.to_string(),
                template_params: template_params.to_vec(),
                params: params.to_vec(),
                body: body.to_vec(),
            });
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(Value::TemplateFn(tmpl), false));
            return Ok(ExecResult::Normal);
        }

        let captured_env = if self.scopes.len() > 1 {
            self.capture_env(body, params)
        } else {
            HashMap::new()
        };
        let fn_val = Rc::new(FnValue {
            name: name.to_string(),
            params: params.to_vec(),
            body: body.to_vec(),
            is_python: self.in_python_module,
            captured_env,
            return_type: return_type.map(|s| s.to_string()),
        });

        if decorators.is_empty() {
            let existing = self
                .scopes
                .last()
                .and_then(|s| s.get(name))
                .map(|v| v.get_value());
            let new_value = match existing {
                Some(Value::Function(prev)) => Value::OverloadedFn(vec![prev, fn_val]),
                Some(Value::OverloadedFn(mut fns)) => {
                    fns.push(fn_val);
                    Value::OverloadedFn(fns)
                }
                _ => Value::Function(fn_val),
            };
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(new_value, false));
        } else {
            let mut value = Value::Function(fn_val);
            for dec_expr in decorators.iter().rev() {
                let dec = self.eval(dec_expr)?;
                value = self.apply_value_call(dec, value, name)?;
            }
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(value, false));
        }
        Ok(ExecResult::Normal)
    }

    /// `gen` 定義を実行してジェネレータ関数値をスコープに登録する。
    pub(crate) fn exec_gen_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        params: &[Param],
        body: &[Stmt],
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateGenFnValue {
                name: name.to_string(),
                template_params: template_params.to_vec(),
                params: params.to_vec(),
                body: body.to_vec(),
            });
            self.scopes.last_mut().unwrap().insert(
                name.to_string(),
                Var::new(Value::TemplateGenFn(tmpl), false),
            );
        } else {
            let captured_env = if self.scopes.len() > 1 {
                self.capture_env(body, params)
            } else {
                HashMap::new()
            };
            let gen_fn = Rc::new(GeneratorFnValue {
                name: name.to_string(),
                params: params.to_vec(),
                body: body.to_vec(),
                captured_env,
            });
            self.scopes.last_mut().unwrap().insert(
                name.to_string(),
                Var::new(Value::GeneratorFn(gen_fn), false),
            );
        }
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Type definitions
    // ---------------------------------------------------------------------------

    /// `trait` 定義を実行してトレイト値をスコープに登録する。アクセス制御情報とフィールド順序も収集する。
    pub(crate) fn exec_trait_def(&mut self, name: &str, body: &[Stmt]) -> Result<ExecResult, String> {
        let mut trait_access: HashMap<String, Accessibility> = HashMap::new();
        let mut field_order: Vec<(String, bool)> = Vec::new();
        for stmt in body {
            if let Stmt::Field {
                name: fname,
                kind,
                access,
                ..
            } = stmt
            {
                field_order.push((fname.clone(), *kind == FieldKind::Mut));
                if *access != Accessibility::Public {
                    trait_access.insert(fname.clone(), access.clone());
                }
            }
            if let Stmt::FnDef {
                name: mname,
                access,
                ..
            } = stmt
            {
                if *access != Accessibility::Public {
                    trait_access.insert(mname.clone(), access.clone());
                }
            }
        }
        if !field_order.is_empty() {
            self.trait_field_order.insert(name.to_string(), field_order);
        }
        if !trait_access.is_empty() {
            self.trait_field_access
                .insert(name.to_string(), trait_access);
        }
        self.declare_var(
            name.to_string(),
            Var::new(Value::Trait(name.to_string()), false),
        );
        Ok(ExecResult::Normal)
    }

    /// `protocol` 定義を実行してプロトコル値をスコープに登録する。
    /// プロトコルは静的型チェック専用で、インスタンス化できない。
    pub(crate) fn exec_protocol_def(&mut self, name: &str, body: &[Stmt]) -> Result<ExecResult, String> {
        // 必須メンバー名を収集（is Protocol 実行時チェック用）
        let mut members: Vec<String> = Vec::new();
        for s in body {
            match s {
                Stmt::Field { name: fname, .. } => members.push(fname.clone()),
                Stmt::FnDef { name: mname, .. } => members.push(mname.clone()),
                _ => {}
            }
        }
        self.protocol_required_members.insert(name.to_string(), members);
        self.declare_var(
            name.to_string(),
            Var::new(Value::Protocol(name.to_string()), false),
        );
        Ok(ExecResult::Normal)
    }

    /// `new_type name: OriginalType` を実行して新しい型をスコープに登録する。
    pub(crate) fn exec_new_type_def(&mut self, name: &str, original: &str) -> Result<ExecResult, String> {
        let orig_val = self
            .get_val(original)
            .ok_or_else(|| format!("NameError: type '{original}' is not defined"))?;
        match orig_val {
            Value::Class(orig_cls) => {
                let new_cls = Rc::new(crate::interpreter::ClassValue {
                    name: name.to_string(),
                    class_id: crate::interpreter::value::alloc_class_id(),
                    bases: orig_cls.bases.clone(),
                    methods: orig_cls.methods.clone(),
                    gen_methods: orig_cls.gen_methods.clone(),
                    field_defaults: orig_cls.field_defaults.clone(),
                    class_vars: orig_cls.class_vars.clone(),
                    field_mutability: orig_cls.field_mutability.clone(),
                    field_index: orig_cls.field_index.clone(),
                    field_count: orig_cls.field_count,
                    field_mutability_vec: orig_cls.field_mutability_vec.clone(),
                    field_access: orig_cls.field_access.clone(),
                    method_access: orig_cls.method_access.clone(),
                    static_method_names: orig_cls.static_method_names.clone(),
                    class_method_names: orig_cls.class_method_names.clone(),
                    static_vars: orig_cls.static_vars.clone(),
                    new_type_base: orig_cls.new_type_base.clone(),
                    is_exception: orig_cls.is_exception,
                    raw_layout: orig_cls.raw_layout.clone(),
                });
                self.declare_var(name.to_string(), Var::new(Value::Class(new_cls), false));
            }
            Value::Type(type_name) => {
                // `new_type Meters: int` → `class Meters: mut value: int` と等価
                let init_body = vec![Stmt::AttrAssign {
                    target: Expr::Attr {
                        object: Box::new(Expr::Ident("self".to_string())),
                        attr: "value".to_string(),
                        span: crate::token::Span::unknown(),
                    },
                    value: Expr::Ident("value".to_string()),
                }];
                let init_fn = Rc::new(FnValue {
                    name: "__init__".to_string(),
                    params: vec![
                        crate::ast::Param {
                            name: "self".to_string(),
                            mutable: true,
                            type_ann: None,
                            default: None,
                            variadic: false,
                        },
                        crate::ast::Param {
                            name: "value".to_string(),
                            mutable: false,
                            type_ann: Some(type_name.clone()),
                            default: None,
                            variadic: false,
                        },
                    ],
                    body: init_body,
                    is_python: false,
                    captured_env: HashMap::new(),
                return_type: None,
                });
                let mut methods = HashMap::new();
                methods.insert("__init__".to_string(), vec![init_fn]);
                let new_cls = Rc::new(crate::interpreter::ClassValue {
                    name: name.to_string(),
                    class_id: crate::interpreter::value::alloc_class_id(),
                    bases: vec![],
                    methods,
                    gen_methods: HashMap::new(),
                    field_defaults: vec![],
                    class_vars: HashMap::new(),
                    field_mutability: HashMap::from([("value".to_string(), true)]),
                    field_index: HashMap::from([("value".to_string(), 0usize)]),
                    field_count: 1,
                    field_mutability_vec: vec![true],
                    field_access: HashMap::new(),
                    method_access: HashMap::new(),
                    static_method_names: HashSet::new(),
                    class_method_names: HashSet::new(),
                    static_vars: HashMap::new(),
                    new_type_base: Some(type_name.clone()),
                    is_exception: false,
                    raw_layout: None,
                });
                self.declare_var(name.to_string(), Var::new(Value::Class(new_cls), false));
            }
            _ => {
                return Err(format!(
                    "TypeError: cannot create new_type from '{original}' — only classes and primitive types are supported"
                ));
            }
        }
        Ok(ExecResult::Normal)
    }

    /// `enum` 定義を実行して列挙型クラスと各バリアントをスコープに登録する。
    pub(crate) fn exec_enum_def(
        &mut self,
        name: &str,
        variants: &[(String, Option<Expr>)],
    ) -> Result<ExecResult, String> {
        // enum_item_Name クラスを生成する（new_type enum_item_Name: int 相当）
        let item_type_name = format!("enum_item_{}", name);
        let init_body = vec![Stmt::AttrAssign {
            target: Expr::Attr {
                object: Box::new(Expr::Ident("self".to_string())),
                attr: "value".to_string(),
                span: crate::token::Span::unknown(),
            },
            value: Expr::Ident("value".to_string()),
        }];
        let init_fn = Rc::new(FnValue {
            name: "__init__".to_string(),
            params: vec![
                crate::ast::Param {
                    name: "self".to_string(),
                    mutable: true,
                    type_ann: None,
                    default: None,
                    variadic: false,
                },
                crate::ast::Param {
                    name: "value".to_string(),
                    mutable: false,
                    type_ann: Some("int".to_string()),
                    default: None,
                    variadic: false,
                },
            ],
            body: init_body,
            is_python: false,
            captured_env: HashMap::new(),
        return_type: None,
        });
        let mut item_methods = HashMap::new();
        item_methods.insert("__init__".to_string(), vec![init_fn]);
        let item_cls_id = crate::interpreter::value::alloc_class_id();
        let item_cls = Rc::new(crate::interpreter::ClassValue {
            name: item_type_name.clone(),
            class_id: item_cls_id,
            bases: vec![],
            methods: item_methods,
            gen_methods: HashMap::new(),
            field_defaults: vec![],
            class_vars: HashMap::new(),
            field_mutability: HashMap::from([("value".to_string(), true)]),
            field_index: HashMap::from([("value".to_string(), 0usize)]),
            field_count: 1,
            field_mutability_vec: vec![true],
            field_access: HashMap::new(),
            method_access: HashMap::new(),
            static_method_names: HashSet::new(),
            class_method_names: HashSet::new(),
            static_vars: HashMap::new(),
            new_type_base: None,
            is_exception: false,
            raw_layout: None,
        });
        self.declare_var(
            item_type_name.clone(),
            Var::new(Value::Class(item_cls.clone()), false),
        );

        // 各バリアントの値を計算し、enum クラスの const クラス変数として登録する
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut next_value: i64 = 0;
        for (variant_name, value_expr) in variants {
            let int_val = if let Some(expr) = value_expr {
                match self.eval(expr)? {
                    Value::Int(n) => n,
                    other => {
                        return Err(format!(
                            "TypeError: enum variant '{}' value must be int, got '{}'",
                            variant_name,
                            self.type_name(&other)
                        ))
                    }
                }
            } else {
                next_value
            };
            next_value = int_val + 1;
            let inst =
                self.instantiate_evaled(item_cls.clone(), vec![(None, Value::Int(int_val), true)])?;
            class_vars.insert(variant_name.clone(), inst);
        }

        let enum_cls = Rc::new(crate::interpreter::ClassValue {
            name: name.to_string(),
            class_id: crate::interpreter::value::alloc_class_id(),
            bases: vec![],
            methods: HashMap::new(),
            gen_methods: HashMap::new(),
            field_defaults: vec![],
            class_vars,
            field_mutability: HashMap::new(),
            field_index: HashMap::new(),
            field_count: 0,
            field_mutability_vec: vec![],
            field_access: HashMap::new(),
            method_access: HashMap::new(),
            static_method_names: HashSet::new(),
            class_method_names: HashSet::new(),
            static_vars: HashMap::new(),
            new_type_base: None,
            is_exception: false,
            raw_layout: None,
        });
        self.declare_var(name.to_string(), Var::new(Value::Class(enum_cls), false));
        Ok(ExecResult::Normal)
    }

    /// `class` 定義を実行してクラス値をスコープに登録する。トレイト継承・フィールド・メソッドを処理する。
    pub(crate) fn exec_class_def(
        &mut self,
        name: &str,
        template_params: &[TemplateParam],
        bases: &[String],
        body: &[Stmt],
        decorators: &[Expr],
    ) -> Result<ExecResult, String> {
        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateClassValue {
                name: name.to_string(),
                template_params: template_params.to_vec(),
                bases: bases.to_vec(),
                body: body.to_vec(),
            });
            self.declare_var(
                name.to_string(),
                Var::new(Value::TemplateClass(tmpl), false),
            );
            return Ok(ExecResult::Normal);
        }

        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
        let mut field_defaults = Vec::new();
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut field_mutability: HashMap<String, bool> = HashMap::new();
        let mut own_field_order: Vec<(String, bool)> = Vec::new();
        let mut own_field_types: Vec<(String, String)> = Vec::new();
        let mut field_access: HashMap<String, Accessibility> = HashMap::new();
        let mut method_access: HashMap<String, Accessibility> = HashMap::new();
        let mut static_method_names: HashSet<String> = HashSet::new();
        let mut class_method_names: HashSet<String> = HashSet::new();
        let mut static_vars: HashMap<String, Rc<RefCell<Value>>> = HashMap::new();

        // 継承トレイトのフィールドアクセス可能性を引き継ぐ
        for base in bases {
            if let Some(trait_acc) = self.trait_field_access.get(base) {
                for (fname, acc) in trait_acc {
                    field_access.insert(format!("{}::{}", base, fname), acc.clone());
                }
            }
        }

        for stmt in body {
            match stmt {
                Stmt::FnDef {
                    name: mname,
                    template_params,
                    params,
                    body: mbody,
                    decorators: mdecs,
                    access: macc,
                    is_static,
                    is_class_method,
                    return_type: mret,
                    ..
                } => {
                    let fn_val = Rc::new(FnValue {
                        name: mname.clone(),
                        params: params.clone(),
                        body: mbody.clone(),
                        is_python: self.in_python_module,
                        captured_env: HashMap::new(),
                        return_type: mret.clone(),
                    });
                    // `__cast__[TypeName]` メソッドはキャスト専用のキー名で格納する。
                    // テンプレートパラメータの名前（具体型名）をキーとして使用する。
                    let storage_name = if mname == "__cast__" && !template_params.is_empty() {
                        format!("__cast__[{}]", template_params[0].name)
                    } else {
                        mname.clone()
                    };
                    if *is_static {
                        static_method_names.insert(storage_name.clone());
                    }
                    if *is_class_method {
                        class_method_names.insert(storage_name.clone());
                    }
                    if *macc != Accessibility::Public {
                        method_access.insert(storage_name.clone(), macc.clone());
                    }
                    if mdecs.is_empty() {
                        methods.entry(storage_name).or_default().push(fn_val);
                    } else {
                        let mut value = Value::Function(fn_val);
                        for dec_expr in mdecs.iter().rev() {
                            let dec = self.eval(dec_expr)?;
                            value = self.apply_value_call(dec, value, mname)?;
                        }
                        match value {
                            Value::Function(f) => {
                                methods.entry(storage_name).or_default().push(f)
                            }
                            other => return Err(format!(
                                "TypeError: method decorator on '{}' must return a function, got '{}'",
                                mname,
                                self.type_name(&other)
                            )),
                        }
                    }
                }
                Stmt::GenDef {
                    name: mname,
                    params,
                    body: mbody,
                    access: macc,
                    ..
                } => {
                    if *macc != Accessibility::Public {
                        method_access.insert(mname.clone(), macc.clone());
                    }
                    gen_methods.insert(
                        mname.clone(),
                        Rc::new(GeneratorFnValue {
                            name: mname.clone(),
                            params: params.clone(),
                            body: mbody.clone(),
                            captured_env: HashMap::new(),
                        }),
                    );
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::Const,
                    default: Some(init),
                    access: facc,
                    ..
                } => {
                    if *facc != Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = self.eval(init)?;
                    class_vars.insert(fname.clone(), val);
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::StaticMut,
                    default,
                    access: facc,
                    ..
                } => {
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
                Stmt::Field {
                    name: fname,
                    kind,
                    type_ann,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let mutable = *kind == FieldKind::Mut;
                    own_field_order.push((fname.clone(), mutable));
                    own_field_types.push((fname.clone(), type_ann.clone()));
                    field_mutability.insert(fname.clone(), mutable);
                    if let Some(init) = default {
                        let val = self.eval(init)?;
                        field_defaults.push((fname.clone(), val, mutable));
                    }
                }
                _ => {}
            }
        }

        let (field_index, field_mutability_vec, field_count) =
            self.build_field_index(&own_field_order, bases);

        // raw ブロックレイアウト（.claude/skills/c-abi-interop/SKILL.md P1）:
        // trait 継承なし・全フィールドがプリミティブ（int/float/C ABI 型）・24 フィールド以下の
        // クラスはフィールドを InstanceData.raw の C ABI レイアウト領域に格納する。
        let raw_layout = if bases.is_empty() && own_field_types.len() == field_count {
            crate::interpreter::value::RawLayout::from_fields(&own_field_types).map(Rc::new)
        } else {
            None
        };

        let cls = Rc::new(crate::interpreter::ClassValue {
            name: name.to_string(),
            class_id: crate::interpreter::value::alloc_class_id(),
            bases: bases.to_vec(),
            methods,
            gen_methods,
            field_defaults,
            class_vars,
            field_mutability,
            field_index,
            field_count,
            field_mutability_vec,
            field_access,
            method_access,
            static_method_names,
            class_method_names,
            static_vars,
            new_type_base: None,
            is_exception: false,
            raw_layout,
        });
        if decorators.is_empty() {
            self.declare_var(name.to_string(), Var::new(Value::Class(cls), false));
        } else {
            let mut value = Value::Class(cls);
            for dec_expr in decorators.iter().rev() {
                let dec = self.eval(dec_expr)?;
                value = self.apply_value_call(dec, value, name)?;
            }
            self.declare_var(name.to_string(), Var::new(value, false));
        }
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Exception handling
    // ---------------------------------------------------------------------------

}
