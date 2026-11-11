// exec/definitions.rs — 定義文の実行: 関数/ジェネレータ定義、トレイト/プロトコル/new_type/enum/クラス定義。

use crate::interpreter::CapturedVar;
use crate::ast::Resolution;
use {
    std::cell::RefCell, std::collections::{HashMap, HashSet},
    std::rc::Rc,
    crate::ast::{
        Accessibility, Expr, FieldKind, Param,
        Stmt, TemplateParam,
    },
    crate::interpreter::{
        ExecResult, FnValue, GeneratorFnValue,
        Interpreter, TemplateClassValue, TemplateFnValue, TemplateGenFnValue, Value, Var,
    },
};

impl Interpreter {
    /// `fn` 定義を実行して関数値をスコープに登録する。テンプレート関数はテンプレート値として格納する。
    /// 同名の既存値と合成して関数値を作る（`fn` のオーバーロード規則・#27）。
    ///
    /// ツリーウォーク（`exec_fn_def`）と VM（`Op::MakeFn`）の**唯一の実装**。
    /// 「同じ判断をする 2 実装は片方を委譲にして畳む」（#22 系列）。
    pub(crate) fn merge_fn_overload(fn_val: Rc<FnValue>, existing: Option<Value>) -> Value {
        match existing {
            Some(Value::Function(prev)) => Value::OverloadedFn(vec![prev, fn_val]),
            Some(Value::OverloadedFn(mut fns)) => {
                fns.push(fn_val);
                Value::OverloadedFn(fns)
            }
            _ => Value::Function(fn_val),
        }
    }

    /// `Op::MakeFn` の実体（#27）: 入れ子 `fn` の関数値を作る。
    ///
    /// キャプチャは 2 種類（ツリーウォークの `capture_env` と同じ区別・#27-d 段階 2b）:
    /// - `captured`（不変）= **生成時点の値を複製**する。呼び出し側が VM フレームの slot から読む。
    /// - `cell_captured`（可変）= **`Rc<RefCell<Value>>` を共有**する。外側フレームのセル表
    ///   （`Op::LoadCell`/`StoreCell` の相手）か `Interpreter::static_cells` から取る。
    ///
    /// ⚠ **自由変数の集合が `capture_env` とずれると閉包変数が黙って消える／共有が切れる**。
    /// VM コンパイラ側の判定は `nested_fn_captures` に 1 本化してある。
    /// `existing` は slot の現在値（オーバーロード合成用。未宣言なら `Value::None`）。
    // 引数が多いのは「クロージャ生成に要る素材」が単に多いから（名前・シグネチャ・本体・
    // 不変キャプチャ・可変キャプチャ・オーバーロード合成用の既存値）。struct に束ねると
    // 呼び出し側（`Op::MakeFn` の `#[inline(never)]` 本体）で組み立てコストが増えるだけ。
    #[allow(clippy::too_many_arguments)]
    /// 入れ子 `gen` のジェネレータ関数値を作る（bug_fix.md B13 段階 E）。
    ///
    /// `Op::MakeFn` の**ジェネレータ側**。捕捉の集め方は入れ子 `fn` と完全に同じで、
    /// 違いは作る値だけ（`FnValue` ではなく `GeneratorFnValue`）。
    /// ⚠ `fn` 側と違いオーバーロード合成は無い（`gen` に多重定義は無い）ので `existing` は見ない。
    pub(crate) fn make_nested_gen_value(
        &mut self,
        name: &str,
        params: &[Param],
        body: &[Stmt],
        captured: Vec<(String, Value)>,
        cell_captured: Vec<(String, std::rc::Rc<std::cell::RefCell<Value>>)>,
        // `outer.<locals>.inner`（`ChunkFnDef::qualname`・タスク 1-14）。
        qualname: Option<Rc<str>>,
    ) -> Value {
        let mut env: HashMap<String, CapturedVar> = HashMap::new();
        // 不変は生成時点の値（複製済み）、可変はセルを共有する。
        for (n, v) in captured {
            env.insert(n, CapturedVar::Immutable(v));
        }
        for (n, c) in cell_captured {
            env.insert(n, CapturedVar::Mutable(c));
        }
        Value::GeneratorFn(Rc::new(GeneratorFnValue {
            globals: self.cur_globals,
            owner_class: self.current_class.clone(),
            name: name.to_string(),
            params: params.to_vec(),
            body: body.to_vec(),
            captured_env: env,
            qualname,
            group: None,
        }))
    }

    pub(crate) fn make_nested_fn_value(
        &mut self,
        name: &str,
        params: &[Param],
        // ⚠ `&[Stmt]` ではなく `Rc` を受け取る（#45）。`Op::MakeFn` は定義サイトが持つ
        // `ChunkFnDef::body` の `Rc` をそのまま渡すので、**実体ごとの AST 複製が消える**
        // （本体 1 文で 0.2µs・1 文増えるごとに +0.19µs）。
        body: std::rc::Rc<[Stmt]>,
        return_type: Option<&str>,
        captured: Vec<(String, Value)>,
        // 可変キャプチャ（#27-d 段階 2b）。**セルを共有**するので値ではなく `Rc` を受け取る。
        cell_captured: Vec<(String, std::rc::Rc<std::cell::RefCell<Value>>)>,
        existing: Value,
        // 定義サイト共有の Chunk（#30）。`Op::MakeFn` は `ChunkFnDef::compiled` を渡す。
        // ツリーウォークの `exec_fn_def` 経由は定義サイトの器を持たないので `None`。
        vm_chunk: Option<crate::vm::chunk::SharedFnChunk>,
        // `outer.<locals>.inner`（`ChunkFnDef::qualname`・タスク 1-14）。
        qualname: Option<Rc<str>>,
    ) -> Value {
        use crate::interpreter::CapturedVar;
        let mut captured_env: HashMap<String, CapturedVar> = captured
            .into_iter()
            .map(|(n, v)| (n, CapturedVar::Immutable(v)))
            .collect();
        for (n, cell) in cell_captured {
            captured_env.insert(n, CapturedVar::Mutable(cell));
        }
        let fn_val = Rc::new(FnValue {
            globals: self.cur_globals,
            owner_class: self.current_class.clone(),
            name: name.to_string(),
            params: params.to_vec(),
            body,
            captured_env,
            return_type: return_type.map(|s| s.to_string()),
            vm_chunk,
            bound: None,
            qualname,
            group: None,
        });
        let existing = match existing {
            Value::None => None,
            v => Some(v),
        };
        Self::merge_fn_overload(fn_val, existing)
    }

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
            });
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(Value::TemplateFn(tmpl), false));
            return Ok(ExecResult::Normal);
        }

        let captured_env = if self.scopes.len() > self.frame_floor {
            self.capture_env(body, params)
        } else {
            HashMap::new()
        };
        let fn_val = Rc::new(FnValue {
            globals: self.cur_globals,
            owner_class: self.current_class.clone(),
            // ⚠ 単相化した関数（`ident[str]`）の値の名前はテンプレートの名前（タスク 2-8）。
            //   実行時の具体化と同じ（traceback の `in ident`）。束縛名は `ident[str]` のまま。
            name: crate::template_subst::display_name(name).to_string(),
            params: params.to_vec(),
            body: std::rc::Rc::from(body),
            captured_env,
            return_type: return_type.map(|s| s.to_string()),
            vm_chunk: None,
            bound: None,
            // 最上位（とツリーウォークで走る本体の中）の関数は名前と同じ。
            qualname: None,
            group: None,
        });

        // 単相化した関数は、このスコープのテンプレートの具体化として登録する（タスク 2-15）。
        // ⚠ 置換で型が区別されるので、単相化した関数どうしが多重定義に合成されることは無い。
        if name.ends_with(']') {
            self.register_mono_instance(name, &Value::Function(Rc::clone(&fn_val)));
        }
        if decorators.is_empty() {
            let existing = self
                .scopes
                .last()
                .and_then(|s| s.get(name))
                .map(|v| v.get_value());
            let new_value = Self::merge_fn_overload(fn_val, existing);
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.to_string(), Var::new(new_value, false));
        } else {
            let mut value = Value::Function(fn_val);
            for dec_expr in decorators.iter().rev() {
                let dec = self.eval_definition_expr(dec_expr)?;
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
                globals: self.cur_globals,
                owner_class: self.current_class.clone(),
                // ⚠ 単相化したジェネレータの値の名前はテンプレートの名前（タスク 2-8）。
                name: crate::template_subst::display_name(name).to_string(),
                params: params.to_vec(),
                body: body.to_vec(),
                captured_env,
                qualname: None,
                group: None,
            });
            if name.ends_with(']') {
                self.register_mono_instance(name, &Value::GeneratorFn(Rc::clone(&gen_fn)));
            }
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
        reject_unexpanded_member_decorators("trait", name, body)?;
        let mut trait_access: HashMap<String, Accessibility> = HashMap::new();
        let mut field_order: Vec<(String, bool, String)> = Vec::new();
        for stmt in body {
            if let Stmt::Field {
                name: fname,
                kind,
                type_ann,
                access,
                ..
            } = stmt
            {
                // ⚠ 型注釈も運ぶ。`build_field_index` が slot 順の `field_tags`
                //   （実行時型判定）を組むのに要る。
                field_order.push((fname.clone(), *kind == FieldKind::Mut, type_ann.clone()));
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
    /// 組み込みの protocol（`builtins.ars` の `Iterable` / `Sized` …・pandas_import_plan.md の 6-7）の必須メンバー。
    /// `Interpreter::new` が `protocol_required_members` の初めの中身にする（型検査器と同じ宣言を読む）。
    pub(crate) fn builtin_protocol_members() -> std::collections::HashMap<String, Vec<String>> {
        crate::type_check::builtins::type_decls()
            .iter()
            .filter_map(|s| match s {
                Stmt::ProtocolDef { name, body, .. } => Some((name.clone(), protocol_member_names(body))),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn exec_protocol_def(&mut self, name: &str, body: &[Stmt]) -> Result<ExecResult, String> {
        reject_unexpanded_member_decorators("protocol", name, body)?;
        // 必須メンバー名を収集（is Protocol 実行時チェック用）
        self.protocol_required_members.insert(name.to_string(), protocol_member_names(body));
        self.declare_var(
            name.to_string(),
            Var::new(Value::Protocol(name.to_string()), false),
        );
        Ok(ExecResult::Normal)
    }

    /// `new_type name: OriginalType` を実行して新しい型をスコープに登録する。
    ///
    /// ⚠ `original` は**型式の文字列**（`int` / `list[int]` / `dict[str,int]`）であって
    /// 変数名とは限らない。`get_val` で引けるのは `int` や利用者クラスのように
    /// **同名の束縛がある**ものだけで、`list[int]` のような具体化済みジェネリクスは
    /// スコープに存在しない。そこで引けなかった場合は `Value::Type` に落として
    /// 下の「プリミティブを包む」経路へ流す。
    ///
    /// ⚠⚠ **落とす条件を「`from_ann` が通る」だけにしてはいけない。** 未知の裸の名前は
    /// `NamedInstance` として通ってしまい、`new_type N: Nonexistent` の `NameError` が
    /// 消える（実際に踏んだ）。⇒ **構造を持つ型（`list[int]` など）だけ**を落とし、
    /// 裸の名前は従来どおり `NameError` にする。
    pub(crate) fn exec_new_type_def(&mut self, name: &str, original: &str) -> Result<ExecResult, String> {
        let structural = crate::type_check::InferredType::from_ann(original)
            .is_some_and(|t| !matches!(t, crate::type_check::InferredType::NamedInstance(_)));
        let orig_val = match self.get_val(original) {
            Some(v) => v,
            None if structural => Value::Type(original.to_string()),
            None => return Err(format!("NameError: type '{original}' is not defined")),
        };
        match orig_val {
            Value::Class(orig_cls) => {
                let new_cls = Rc::new(crate::interpreter::ClassValue {
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
                    ..crate::interpreter::ClassValue::synthetic(name.to_string(), crate::interpreter::value::alloc_class_id())
                });
                self.declare_var(name.to_string(), Var::new(Value::Class(new_cls), false));
            }
            Value::Type(type_name) => {
                // `new_type Meters: int` → `class Meters: mut value: int` と等価
                let init_body = vec![Stmt::AttrAssign {
                    target: Expr::Attr {
                        object: Box::new(Expr::Ident { name: "self".to_string(), node_id: 0, res: Resolution::Unresolved, span: crate::token::Span::unknown() }),
                        attr: "value".to_string(),
                        span: crate::token::Span::unknown(),
                        attr_span: crate::token::Span::unknown(),
                        cache: Default::default(),
                        node_id: 0, // #16: 合成/変換コード（注釈対象外）
                    },
                    value: Expr::Ident { name: "value".to_string(), node_id: 0, res: Resolution::Unresolved, span: crate::token::Span::unknown() },
                    span: crate::token::Span::unknown(),
                }];
                let init_fn = Rc::new(FnValue {
                    globals: self.cur_globals,
                    owner_class: None,
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
                    body: std::rc::Rc::from(init_body),
                    captured_env: HashMap::new(),
                return_type: None,
                vm_chunk: None,
                bound: None,
                qualname: None,
                    group: None,
                });
                let mut methods = HashMap::new();
                methods.insert("__init__".to_string(), vec![init_fn]);
                let new_cls = Rc::new(crate::interpreter::ClassValue {
                    methods,
                    field_mutability: HashMap::from([("value".to_string(), true)]),
                    field_index: HashMap::from([("value".to_string(), 0usize)]),
                    field_count: 1,
                    field_mutability_vec: vec![true],
                    new_type_base: Some(type_name.clone()),
                    ..crate::interpreter::ClassValue::synthetic(name.to_string(), crate::interpreter::value::alloc_class_id())
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
        let enum_cls = self.build_enum_classes(name, variants)?;
        self.declare_var(name.to_string(), Var::new(Value::Class(enum_cls), false));
        Ok(ExecResult::Normal)
    }

    /// VM の `Op::EnumDef` 用（#68）。`Name` クラスを**値として返す**
    /// （呼び出し側がフレームの slot へ入れる）。名前での宣言はしない。
    pub(crate) fn vm_enum_def(
        &mut self,
        name: &str,
        variants: &[(String, Option<Expr>)],
    ) -> Result<Value, String> {
        Ok(Value::Class(self.build_enum_classes(name, variants)?))
    }

    /// `enum` 定義から、メンバーのクラスと `Name` クラスの 2 つを組み立てて `Name` クラスを返す（#68）。
    ///
    /// - メンバーのクラス … 名前は **enum と同じ `Name`**、`value` フィールドを 1 つ持つ。
    ///   `ClassValue::enum_of` が `Some(Name)`。各メンバー（`Name.X`）はこのクラスのインスタンス。
    /// - `Name` クラス … メンバーを `const` クラス変数として持つ（`Name.X` で引く）。
    ///
    /// ⚠⚠ **メンバーのクラス名は enum の名前**（`implementation_plans/enum_member_type_plan.md` 1-2）。
    ///   以前は `enum_item_Name` という別の名前で、`x is Name` / `case Name:` / `mustbe Name` /
    ///   `Name` 型のフィールドがメンバーを受けなかった（`Name` 型の値が実行時に存在しなかった）。
    ///   メンバーのクラスは**名前で宣言しない**（`enum_item_Name` は廃止した名前）。
    ///
    /// **スコープへの登録はしない**。呼び出し元が記憶域を決める:
    /// - ツリーウォーク（`exec_enum_def`）は `Name` を `declare_var` する（最上位・モジュール本体）。
    /// - VM（`Op::EnumDef`）は `Name` を**フレームの slot** へ入れる（関数内の `enum`）。
    ///   リゾルバの `collect_base_decls` が `Name` に base slot を採番するので、
    ///   読みは `LoadLocal` になる＝ slot に入れないと**別の変数を読む**。
    ///
    /// ⚠ **組み立てと登録を分けたのは #68 の実バグのため**。分ける前は「組み立て」と「`declare_var`」が
    /// 一体だったので、VM から呼ぶと `Name` が slot ではなく**呼び出し元のスコープ**へ
    /// 入ってしまい、載せようが無かった（＝ `compile_stmt` が bail し `VmForceError`）。
    fn build_enum_classes(
        &mut self,
        name: &str,
        variants: &[(String, Option<Expr>)],
    ) -> Result<Rc<crate::interpreter::ClassValue>, String> {
        // メンバーのクラス（`value: int` を持つだけ）。`is tags.Name` のためにモジュール名も付ける。
        let module_name = self.global_module_names.get(self.cur_globals as usize).cloned().flatten();
        let init_body = vec![Stmt::AttrAssign {
            target: Expr::Attr {
                object: Box::new(Expr::Ident { name: "self".to_string(), node_id: 0, res: Resolution::Unresolved, span: crate::token::Span::unknown() }),
                attr: "value".to_string(),
                span: crate::token::Span::unknown(),
                attr_span: crate::token::Span::unknown(),
                cache: Default::default(),
                node_id: 0, // #16: 合成/変換コード（注釈対象外）
            },
            value: Expr::Ident { name: "value".to_string(), node_id: 0, res: Resolution::Unresolved, span: crate::token::Span::unknown() },
            span: crate::token::Span::unknown(),
        }];
        let init_fn = Rc::new(FnValue {
            globals: self.cur_globals,
            owner_class: None,
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
            body: std::rc::Rc::from(init_body),
            captured_env: HashMap::new(),
        return_type: None,
        vm_chunk: None,
        bound: None,
        qualname: None,
            group: None,
        });
        let mut item_methods = HashMap::new();
        item_methods.insert("__init__".to_string(), vec![init_fn]);
        let item_cls_id = crate::interpreter::value::alloc_class_id();
        // ⚠⚠ **`value` は不変**。メンバーは値なので中身を書き換えられない（`Color.BLUE.value = 5` は
        //    `TypeError`）。以前は可変で、**共有のメンバーそのものが書き換わっていた**。
        //    `__init__` の中の最初の代入（`self.value = value`）は `let` フィールドと同じく通る。
        let item_cls = Rc::new(crate::interpreter::ClassValue {
            methods: item_methods,
            field_mutability: HashMap::from([("value".to_string(), false)]),
            field_index: HashMap::from([("value".to_string(), 0usize)]),
            field_count: 1,
            field_mutability_vec: vec![false],
            module_name: module_name.clone(),
            enum_of: Some(name.to_string()),
            ..crate::interpreter::ClassValue::synthetic(name.to_string(), item_cls_id)
        });
        // ⚠ ここで `declare_var` はしない（#68）。記憶域は呼び出し元が決める。

        // 各バリアントの値を計算し、enum クラスの const クラス変数として登録する
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut next_value: i64 = 0;
        for (variant_name, value_expr) in variants {
            let int_val = if let Some(expr) = value_expr {
                match self.eval_definition_expr(expr)? {
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

        Ok(Rc::new(crate::interpreter::ClassValue {
            class_vars,
            module_name,
            ..crate::interpreter::ClassValue::synthetic(name.to_string(), crate::interpreter::value::alloc_class_id())
        }))
    }

    /// クラスのメソッドの `__qualname__`（`C.m`・python_builtins_plan.md のタスク 1-14）。
    ///
    /// 具体化したクラス（`Box[int]`）はテンプレートの名前（`Box.get`）。
    fn method_qualname(&self, class_name: &str, method: &str) -> Rc<str> {
        Rc::from(format!("{}.{method}", crate::template_subst::display_name(class_name)))
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
        reject_unexpanded_member_decorators("class", name, body)?;

        if !template_params.is_empty() {
            let tmpl = Rc::new(TemplateClassValue {
                name: name.to_string(),
                template_params: template_params.to_vec(),
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
        let mut own_field_order: Vec<(String, bool, String)> = Vec::new();
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
                        globals: self.cur_globals,
                        owner_class: None,
                        name: mname.clone(),
                        params: params.clone(),
                        body: std::rc::Rc::from(&mbody[..]),
                        captured_env: HashMap::new(),
                        return_type: mret.clone(),
                        vm_chunk: None,
                        bound: None,
                        qualname: Some(self.method_qualname(name, mname)),
                        group: None,
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
                            let dec = self.eval_definition_expr(dec_expr)?;
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
                            globals: self.cur_globals,
                            owner_class: None,
                            name: mname.clone(),
                            params: params.clone(),
                            body: mbody.clone(),
                            captured_env: HashMap::new(),
                            qualname: Some(self.method_qualname(name, mname)),
                            group: None,
                        }),
                    );
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::Const,
                    type_ann,
                    default: Some(init),
                    access: facc,
                    ..
                } => {
                    if *facc != Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = self.eval_definition_expr(init)?;
                    // 案 B（0-B2 と同じ規則）: `float` 注釈なら昇格する。
                    // ⚠ `const` はクラス変数なので `store_field` の raw レイアウト昇格を
                    //   通らない。ここで揃えないと `const F: float = 3` が `3` を返す（実測）。
                    let val = crate::interpreter::exec::vars::coerce_binding(Some(type_ann), val);
                    class_vars.insert(fname.clone(), val);
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::StaticMut,
                    type_ann,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = if let Some(init) = default {
                        // `const` と同じ理由で昇格を揃える（`static_cells` も raw レイアウト外）。
                        let v = self.eval_definition_expr(init)?;
                        crate::interpreter::exec::vars::coerce_binding(Some(type_ann), v)
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
                    own_field_order.push((fname.clone(), mutable, type_ann.clone()));
                    own_field_types.push((fname.clone(), type_ann.clone()));
                    field_mutability.insert(fname.clone(), mutable);
                    if let Some(init) = default {
                        let val = self.eval_definition_expr(init)?;
                        field_defaults.push((fname.clone(), val, mutable));
                    }
                }
                _ => {}
            }
        }

        let (field_index, field_mutability_vec, field_tags, field_checks, field_count) =
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
            // モジュールで定義したクラスはモジュールの名前を持つ（`tags.Tag` の判定・10-8）。
            // ⚠ 関数の中の定義でも、関数は定義したモジュールの大域で走る（`switch_globals`）ので正しく引ける。
            module_name: self.global_module_names.get(self.cur_globals as usize).cloned().flatten(),
            // 単相化したクラスは具体化の名前も持つ（`x is Stack[int]` の判定・10-10）。
            // ⚠ 綴りはパーサの正規形（空白なし・`Pair[str,int]`）に揃える。展開器の束縛名は
            //   `Pair[str, int]`（`template_subst::instance_name`）で、そのままだと一致しない（実測）。
            instance_name: name
                .ends_with(']')
                .then(|| name.chars().filter(|c| !c.is_whitespace()).collect()),
            bases: bases.to_vec(),
            methods,
            gen_methods,
            field_defaults,
            class_vars,
            field_mutability,
            field_index,
            field_count,
            field_mutability_vec,
            field_tags,
            field_checks,
            field_access,
            method_access,
            static_method_names,
            class_method_names,
            static_vars,
            raw_layout,
            // ⚠⚠ 単相化したクラス（`Box[int]`）の値の名前はテンプレートの名前（タスク 2-8）。
            //   以前の実行時の具体化（削除済み）と同じにする —— `<Box object>` の表示・
            //   型引数を省いた `Box` 注釈との照合（`mut bare: Box = Box[str](..)`）が変わらないように。
            //   束縛名は `Box[int]` のまま（`declare_var` は `name` を使う）。
            ..crate::interpreter::ClassValue::synthetic(
                crate::template_subst::display_name(name).to_string(),
                crate::interpreter::value::alloc_class_id(),
            )
        });
        let mut value = Value::Class(cls);
        for dec_expr in decorators.iter().rev() {
            let dec = self.eval_definition_expr(dec_expr)?;
            value = self.apply_value_call(dec, value, name)?;
        }
        // 単相化したクラスは、このスコープのテンプレートの具体化として登録する（タスク 2-15）。
        if name.ends_with(']') {
            self.register_mono_instance(name, &value);
        }
        self.declare_var(name.to_string(), Var::new(value, false));
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // Exception handling
    // ---------------------------------------------------------------------------

}

/// クラス／トレイト／プロトコル本体に残った `!装飾子` を明示エラーにする（タスク 1-4）。
///
/// ⚠⚠ **本体の文は `exec` を通らない**（フィールド・メソッドとして読み取られるだけ）ので、
/// [`Stmt::MetaDecorated`] を放っておくと**黙って無視される**。「装飾子を書いたのに何も
/// 起きない」は最悪の失敗形なので、展開器（Phase 2）が消すまではここで止める。
fn reject_unexpanded_member_decorators(
    kind: &str,
    name: &str,
    body: &[Stmt],
) -> Result<(), String> {
    for st in body {
        if let Stmt::MetaDecorated { decorators, .. } = st {
            let what = match decorators.first() {
                Some(Expr::Ident { name: d, .. }) => format!("`!{d}`"),
                Some(_) => "this decorator".to_string(),
                None => "a decorator".to_string(),
            };
            return Err(format!(
                "MetaError: {what} on a member of {kind} '{name}' was not expanded \
                 (the compile-time expander does not run in the REPL or the debugger, and a module that uses metafunctions must be imported at the top level)"
            ));
        }
    }
    Ok(())
}

/// protocol の必須メンバーの名前（フィールドとメソッド）。`exec_protocol_def` と組み込みの protocol が共有する。
fn protocol_member_names(body: &[Stmt]) -> Vec<String> {
    body.iter()
        .filter_map(|s| match s {
            Stmt::Field { name, .. } | Stmt::FnDef { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}
