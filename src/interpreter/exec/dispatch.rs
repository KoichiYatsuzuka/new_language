// exec/dispatch.rs — exec のメインディスパッチャ: 文の種類に応じて各専用メソッドへ委譲する。

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
    /// 文（`Stmt`）を実行して `ExecResult` を返す。各 Stmt バリアントを専用メソッドに委譲する。
    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        // Step-mode check: pause before this statement if the debugger asked us to.
        // Skip the check when we're already inside a break_point (would re-enter).
        if crate::interpreter::debugger::DBG_MODE.with(|m| *m.borrow() != DbgMode::Inactive) {
            if let Some(span) = self.should_pause_at(stmt) {
                self.exec_breakpoint(&span)?;
            }
        }

        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, _, expr) => self.exec_let(name, expr),
            Stmt::Const(name, _, expr) => {
                if name != "_" && self.get_var(name).is_some() {
                    return Err(format!("NameError: variable '{name}' is already declared"));
                }
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, _, expr) => {
                if name != "_" && self.get_var(name).is_some() {
                    return Err(format!("NameError: variable '{name}' is already declared"));
                }
                let value = Self::deep_copy_value(self.eval(expr)?);
                self.declare_var(name.clone(), Var::new(value, true));
                Ok(ExecResult::Normal)
            }
            Stmt::LetTuple { targets, value, .. } => self.exec_let_tuple(targets, value),
            Stmt::Static(name, expr, span) => self.exec_static_var(name, expr, span),
            Stmt::Assign { name, value, slot, .. } => {
                // スロットキャッシュ命中: スコープ検索なしの直接セル書き込み
                if let Some(idx) = slot.get(self.slot_epoch) {
                    let value = self.eval(value)?;
                    *self.global_slot_cells[idx].borrow_mut() = value;
                    return Ok(ExecResult::Normal);
                }
                let value = self.eval(value)?;
                self.assign_var(name, value)?;
                self.try_fill_slot(name, slot);
                Ok(ExecResult::Normal)
            }
            Stmt::AttrAssign { target, value } => {
                let rhs = self.eval(value)?;
                self.attr_assign(target, rhs)?;
                Ok(ExecResult::Normal)
            }
            Stmt::AttrCompoundAssign { target, op, value } => {
                let rhs = self.eval(value)?;
                let lhs = self.eval(target)?;
                let result = self.apply_binop_dyn(op, lhs, rhs)?;
                self.attr_assign(target, result)?;
                Ok(ExecResult::Normal)
            }
            Stmt::CompoundAssign {
                name, op, value, slot, ..
            } => self.exec_compound_assign(name, op, value, slot),
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Field { .. } => Ok(ExecResult::Normal),
            Stmt::Break => {
                if !LOOP_DEPTH.with(|d| *d.borrow() > 0) {
                    return Err("SyntaxError: 'break' outside for/while loop".to_string());
                }
                Ok(ExecResult::Break)
            }
            Stmt::Continue => {
                if !LOOP_DEPTH.with(|d| *d.borrow() > 0) {
                    return Err("SyntaxError: 'continue' outside for/while loop".to_string());
                }
                Ok(ExecResult::Continue)
            }
            Stmt::Return(expr) => {
                let val = match expr {
                    Some(e) => self.eval(e)?,
                    None => Value::None,
                };
                Ok(ExecResult::Return(val))
            }
            Stmt::BlockReturn(expr, _span) => {
                let val = self.eval(expr)?;
                let expected =
                    BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow().last().cloned().flatten());
                if let Some(ann) = expected {
                    self.check_block_return_type(&val, &ann)?;
                }
                Ok(ExecResult::BlockReturn(val))
            }
            Stmt::LoopYield(expr) => self.exec_loop_yield(expr),
            Stmt::If {
                branches,
                else_body,
            } => self.exec_if_stmt(branches, else_body),
            Stmt::Match { subject, arms, .. } => self.exec_match_stmt(subject, arms),
            Stmt::While { cond, body } => self.exec_while_stmt(cond, body),
            Stmt::For {
                targets,
                iter,
                body,
            } => self.exec_for_stmt(targets, iter, body),
            Stmt::Block(body) => self.exec_block_stmt(body),
            Stmt::FnDef {
                name,
                template_params,
                params,
                body,
                decorators,
                return_type,
                ..
            } => self.exec_fn_def(name, template_params, params, body, decorators, return_type.as_deref()),
            Stmt::Yield(expr) => {
                let val = self.eval(expr)?;
                GENERATOR_YIELDS.with(|y| {
                    if let Some(yields) = y.borrow_mut().as_mut() {
                        yields.push(val.clone());
                    }
                });
                Ok(ExecResult::Normal)
            }
            Stmt::GenDef {
                name,
                template_params,
                params,
                body,
                ..
            } => self.exec_gen_def(name, template_params, params, body),
            Stmt::TraitDef { name, body, .. } => self.exec_trait_def(name, body),
            Stmt::ProtocolDef { name, body } => self.exec_protocol_def(name, body),
            Stmt::NewTypeDef { name, original } => self.exec_new_type_def(name, original),
            Stmt::EnumDef { name, variants } => self.exec_enum_def(name, variants),
            Stmt::ClassDef {
                name,
                template_params,
                bases,
                body,
                decorators,
            } => self.exec_class_def(name, template_params, bases, body, decorators),
            Stmt::Freeze(name, span) => self.exec_freeze(name, span),
            Stmt::Raise { exc, span } => self.exec_raise(exc, span),
            Stmt::Try {
                body,
                handlers,
                finally_body,
            } => self.exec_try(body, handlers, finally_body),
            Stmt::Import {
                lang,
                module,
                with_file,
                alias,
                body,
            } => {
                let ns = if lang == "cpp-dll" || lang == "cpp-lib" {
                    let file_path = module.first().map(|s| s.as_str()).unwrap_or("");
                    let cache_key = (lang.clone(), PathBuf::from(file_path));
                    if let Some(ModuleState::Loaded(cached)) =
                        self.module_cache.get(&cache_key).cloned()
                    {
                        cached
                    } else {
                        self.module_cache
                            .insert(cache_key.clone(), ModuleState::Loading);
                        let ns = self.load_cpp_module(lang, file_path, with_file.as_deref())?;
                        self.module_cache
                            .insert(cache_key, ModuleState::Loaded(ns.clone()));
                        ns
                    }
                } else if lang == "cs-dll" {
                    let stub_ns = self.exec_module(lang, module, body)?;
                    // Locate the NativeAOT bridge DLL next to the managed DLL.
                    let managed_name = module.last().unwrap();
                    let native_dll_name = format!("{managed_name}_native.dll");
                    let sub_dir: PathBuf = module[..module.len().saturating_sub(1)].iter().collect();
                    let bridge_path = {
                        let mut found: Option<PathBuf> = None;
                        for search_dir in &self.python_search_dirs {
                            let c = search_dir.join(&sub_dir).join(&native_dll_name);
                            if c.exists() { found = Some(c); break; }
                            let c2 = search_dir.join(&native_dll_name);
                            if c2.exists() { found = Some(c2); break; }
                        }
                        if found.is_none() {
                            let c = sub_dir.join(&native_dll_name);
                            if c.exists() { found = Some(c); }
                        }
                        if found.is_none() {
                            let c = PathBuf::from(&native_dll_name);
                            if c.exists() { found = Some(c); }
                        }
                        found.map(std::rc::Rc::new)
                    };
                    if let Some(ref bp) = bridge_path {
                        if let Err(e) = crate::interpreter::cs_dll_runtime::load_bridge(bp.as_ref()) {
                            eprintln!("Warning: cs-dll bridge not loaded: {e}");
                        } else {
                            let bp_str = bp.to_string_lossy().into_owned();
                            let mut patched = (*stub_ns).clone();
                            for val in patched.members.values_mut() {
                                if let Value::Class(cls) = val {
                                    let mut new_cls = cls.deep_clone();
                                    new_cls.class_vars.insert(
                                        "__cs_bridge_path__".to_string(),
                                        Value::Str(bp_str.clone()),
                                    );
                                    *val = Value::Class(std::rc::Rc::new(new_cls));
                                }
                            }
                            return {
                                let bind_name = alias.clone().unwrap_or_else(|| module.last().unwrap().clone());
                                self.declare_var(bind_name, Var::new(Value::Namespace(std::rc::Rc::new(patched)), false));
                                Ok(ExecResult::Normal)
                            };
                        }
                    }
                    stub_ns
                } else if lang == "cs-proc" {
                    let stub_ns = self.exec_module(lang, module, body)?;
                    // Locate the cs-proc host executable.
                    // Searches for {Name}_proc.exe first, then {Name}.exe.
                    let managed_name = module.last().unwrap();
                    let sub_dir: PathBuf = module[..module.len().saturating_sub(1)].iter().collect();
                    let proc_path = {
                        let candidates_names = [
                            format!("{managed_name}_proc.exe"),
                            format!("{managed_name}.exe"),
                        ];
                        let mut found: Option<PathBuf> = None;
                        'outer: for name in &candidates_names {
                            for search_dir in &self.python_search_dirs {
                                let c = search_dir.join(&sub_dir).join(name);
                                if c.exists() { found = Some(c); break 'outer; }
                                let c2 = search_dir.join(name);
                                if c2.exists() { found = Some(c2); break 'outer; }
                                // Single-segment: also try source_dir/name_dir/exe_name
                                if module.len() == 1 {
                                    let c3 = search_dir.join(managed_name).join(name);
                                    if c3.exists() { found = Some(c3); break 'outer; }
                                }
                            }
                            let c = sub_dir.join(name);
                            if c.exists() { found = Some(c); break; }
                            // Single-segment CWD fallback: managed_name/exe_name
                            if module.len() == 1 {
                                let c2 = PathBuf::from(managed_name).join(name);
                                if c2.exists() { found = Some(c2); break; }
                            }
                            let c = PathBuf::from(name);
                            if c.exists() { found = Some(c); break; }
                        }
                        found
                    };
                    if let Some(ref pp) = proc_path {
                        match crate::interpreter::cs_proc_runtime::launch_proc(pp.as_ref()) {
                            Err(e) => eprintln!("Warning: cs-proc host not started: {e}"),
                            Ok(()) => {
                                let pp_str = pp.to_string_lossy().into_owned();
                                let mut patched = (*stub_ns).clone();
                                for val in patched.members.values_mut() {
                                    if let Value::Class(cls) = val {
                                        let mut new_cls = cls.deep_clone();
                                        new_cls.class_vars.insert(
                                            "__cs_proc_path__".to_string(),
                                            Value::Str(pp_str.clone()),
                                        );
                                        *val = Value::Class(std::rc::Rc::new(new_cls));
                                    }
                                }
                                return {
                                    let bind_name = alias.clone().unwrap_or_else(|| module.last().unwrap().clone());
                                    self.declare_var(bind_name, Var::new(Value::Namespace(std::rc::Rc::new(patched)), false));
                                    Ok(ExecResult::Normal)
                                };
                            }
                        }
                    }
                    stub_ns
                } else if lang == "js-proc" {
                    // import[js-proc]: Node.js IPC サブプロセス経由で JS モジュールを呼び出す。
                    // 1. ar_config.json から node_path と bridge_script を読み込む
                    // 2. ブリッジプロセスを起動（キャッシュ済みなら再利用）
                    // 3. list 操作でモジュールのエクスポート関数名を取得
                    // 4. 各関数を Value::JsProcFn としてネームスペースに登録
                    let cfg = find_js_config(&self.python_search_dirs);
                    match cfg {
                        Err(e) => {
                            eprintln!("Warning: js-proc: {e}");
                            self.exec_module(lang, module, body)?
                        }
                        Ok((node_exe, bridge_script, bridge_root)) => {
                            let bridge_key = bridge_script
                                .canonicalize()
                                .unwrap_or_else(|_| bridge_script.clone())
                                .to_string_lossy()
                                .into_owned();

                            match crate::interpreter::js_proc_runtime::launch_proc(
                                &node_exe, &bridge_script, &bridge_root,
                            ) {
                                Err(e) => {
                                    eprintln!("Warning: js-proc bridge not started: {e}");
                                    self.exec_module(lang, module, body)?
                                }
                                Ok(()) => {
                                    let module_name = module.join("/");
                                    let fn_names = crate::interpreter::js_proc_runtime::list_functions(
                                        &bridge_key, &module_name,
                                    ).unwrap_or_else(|e| {
                                        eprintln!("Warning: js-proc list_functions: {e}");
                                        vec![]
                                    });

                                    let mut members = std::collections::HashMap::new();
                                    for fn_name in fn_names {
                                        members.insert(fn_name.clone(), Value::JsProcFn {
                                            bridge_key:  bridge_key.clone(),
                                            module_name: module_name.clone(),
                                            fn_name,
                                        });
                                    }
                                    let ns = std::rc::Rc::new(crate::interpreter::NamespaceData {
                                        name: module.join("."),
                                        members,
                                    });
                                    let bind_name = alias.clone()
                                        .unwrap_or_else(|| module.last().unwrap().clone());
                                    self.declare_var(
                                        bind_name,
                                        Var::new(Value::Namespace(ns), false),
                                    );
                                    return Ok(ExecResult::Normal);
                                }
                            }
                        }
                    }
                } else {
                    self.exec_module(lang, module, body)?
                };
                // Default bind name: for cpp imports use the file stem; otherwise last module segment
                let bind_name = alias.clone().unwrap_or_else(|| {
                    if lang == "cpp-dll" || lang == "cpp-lib" {
                        let path = module.first().map(|s| s.as_str()).unwrap_or("lib");
                        std::path::Path::new(path)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("lib")
                            .to_string()
                    } else {
                        module.last().unwrap().clone()
                    }
                });
                self.declare_var(bind_name, Var::new(Value::Namespace(ns), false));
                Ok(ExecResult::Normal)
            }
            Stmt::FromImport {
                lang,
                module,
                with_file: _,
                names,
                body,
            } => {
                let ns = self.exec_module(lang, module, body)?;
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let val = ns.members.get(orig_name.as_str()).cloned().ok_or_else(|| {
                        format!(
                            "ImportError: cannot import name '{}' from '{}'",
                            orig_name,
                            module.join(".")
                        )
                    })?;
                    self.declare_var(bind_name, Var::new(val, false));
                }
                Ok(ExecResult::Normal)
            }
            Stmt::AsyncAssign { target, stmts, .. } => self.exec_async_assign(target, stmts),
            Stmt::BreakPoint { span } => self.exec_breakpoint(span),
            Stmt::DebugLet(name, expr) => {
                let value = self.eval(expr)?;
                self.dbg_vars.insert(name.clone(), Var::new(value, false));
                Ok(ExecResult::Normal)
            }
            Stmt::EventSubscribe {
                source,
                handler,
                is_once,
                is_async,
                ..
            } => self.exec_event_subscribe(source, handler, *is_once, *is_async),
            Stmt::EventUnsubscribe {
                source, handler, ..
            } => self.exec_event_unsubscribe(source, handler),
        }
    }

    // ---------------------------------------------------------------------------
    // Variable declarations & assignment
    // ---------------------------------------------------------------------------

}
