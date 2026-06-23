// eval.rs — 式の評価・attr_assign (eval / attr_assign)
//
// `Interpreter::eval` が式（`Expr`）を再帰的にツリーウォークして `Value` を返す。
// 属性への代入（`self.x = v` や `d[k] = v`）は `attr_assign` が担当する。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::{Accessibility, BinOp, CallArg, Expr, MatchArm, MatchPattern};
use crate::token::Span;

use super::{
    ByteModeRust, DictData, ExecResult, FileData, FileOpenModeRust, GeneratorState, Interpreter,
    NativeFnRef, SliceValue, TupleData, Value, Var, BLOCK_RETURN_EXPECTED_TYPE, BLOCK_YIELDS,
    BREAK_SENTINEL, LOOP_DEPTH, RAISE_SENTINEL,
};

/// ヘルパー: セットに要素を重複なしで追加する。
fn set_insert(set: &mut Vec<Value>, item: Value, interp: &Interpreter) {
    if !set.iter().any(|v| interp.values_eq(v, &item)) {
        set.push(item);
    }
}

// ---------------------------------------------------------------------------
// スライス計算ヘルパー
// ---------------------------------------------------------------------------

/// step=1 スライス代入用: begin 境界を `[0, len]` にクランプして `usize` で返す。
fn normalize_slice_bound_start(begin: Option<i64>, len: i64) -> usize {
    match begin {
        None => 0,
        Some(i) if i < 0 => (i + len).max(0) as usize,
        Some(i) => i.min(len) as usize,
    }
}

/// step=1 スライス代入用: end 境界を `[0, len]` にクランプして `usize` で返す。
fn normalize_slice_bound_stop(end: Option<i64>, len: i64) -> usize {
    match end {
        None => len as usize,
        Some(i) if i < 0 => (i + len).max(0) as usize,
        Some(i) => i.min(len) as usize,
    }
}

/// `Optional[Index]` 値から i64 インデックスを取り出す。None または Value::None → None。
fn index_val_to_i64(val: &Option<Value>) -> Option<i64> {
    match val {
        None => None,
        Some(v) => value_as_index(v),
    }
}

/// `Value` を整数インデックスとして解釈する。
/// `Value::Int(n)` または `Index` インスタンス（`.value` フィールドが `int`）を受け入れる。
fn value_as_index(val: &Value) -> Option<i64> {
    match val {
        Value::Int(n) => Some(*n),
        Value::Instance(inst) => {
            let b = inst.borrow();
            if b.class.name == "Index" {
                if let Some((Value::Int(n), _)) = b.fields.get("value") {
                    return Some(*n);
                }
            }
            None
        }
        _ => None,
    }
}

/// Python 互換のスライスインデックスリストを返す（`obj[begin:end:step]`）。
fn compute_slice_indices(len: i64, begin: Option<i64>, end: Option<i64>, step: i64) -> Vec<usize> {
    let (start, stop) = if step > 0 {
        let s = match begin {
            None => 0,
            Some(i) if i < 0 => (i + len).max(0),
            Some(i) => i.min(len),
        };
        let e = match end {
            None => len,
            Some(i) if i < 0 => (i + len).max(0),
            Some(i) => i.min(len),
        };
        (s, e)
    } else {
        let s = match begin {
            None => len - 1,
            Some(i) if i < 0 => (i + len).max(-1),
            Some(i) => i.min(len - 1),
        };
        let e = match end {
            None => -(len + 1),
            Some(i) if i < 0 => (i + len).max(-1),
            Some(i) => i.min(len - 1),
        };
        (s, e)
    };

    let mut result = Vec::new();
    let mut i = start;
    loop {
        if step > 0 {
            if i >= stop {
                break;
            }
        } else {
            if i <= stop || i < 0 {
                break;
            }
        }
        if i >= 0 && i < len {
            result.push(i as usize);
        }
        i += step;
    }
    result
}

// ---------------------------------------------------------------------------
// open() / close() ヘルパー
// ---------------------------------------------------------------------------

/// str または path インスタンスからファイルパス文字列を取り出す。
fn extract_path_str(val: &Value) -> Result<String, String> {
    match val {
        Value::Str(s) => Ok(s.clone()),
        Value::Instance(inst_rc) => {
            let inst = inst_rc.borrow();
            if inst.class.name == "path" {
                if let Some((Value::Str(s), _)) = inst.fields.get("value") {
                    return Ok(s.clone());
                }
            }
            Err(format!(
                "TypeError: open() 'file_path' must be str or path, got instance of '{}'",
                inst.class.name
            ))
        }
        other => Err(format!(
            "TypeError: open() 'file_path' must be str or path, got '{}'",
            match other {
                Value::Int(_) => "int",
                Value::Float(_) => "float",
                Value::Bool(_) => "bool",
                Value::None => "NoneType",
                _ => "other",
            }
        )),
    }
}

/// enum インスタンスの整数値を取り出す。クラス名が一致しない場合はエラー。
fn extract_enum_int(val: &Value, expected_class: &str) -> Result<i64, String> {
    if let Value::Instance(inst_rc) = val {
        let inst = inst_rc.borrow();
        if inst.class.name == expected_class {
            if let Some((Value::Int(n), _)) = inst.fields.get("value") {
                return Ok(*n);
            }
        }
        return Err(format!(
            "TypeError: expected {expected_class} instance, got instance of '{}'",
            inst.class.name
        ));
    }
    Err(format!("TypeError: expected {expected_class} instance"))
}

/// 位置引数とキーワード引数のどちらからでも値を取り出すヘルパー。
fn get_arg<'a>(
    pos: &'a [Value],
    kw: &'a std::collections::HashMap<String, Value>,
    idx: usize,
    name: &str,
) -> Option<&'a Value> {
    kw.get(name).or_else(|| pos.get(idx))
}

impl Interpreter {
    /// メンバー（フィールド or メソッド）のアクセス可能性を検査する。
    ///
    /// `class` の `field_access` / `method_access` マップで `member_key` を検索し、
    /// アクセス制御に違反する場合は `Err(AccessError: ...)` を返す。
    ///
    /// - `Public`    : 常に OK。
    /// - `Private`   : `self.current_class.name == class.name` のときのみ OK。
    /// - `Protected` : `self.current_class` が同じクラス、またはそのクラスを基底に持つとき OK。
    fn check_member_access(
        &self,
        class: &super::ClassValue,
        member_key: &str,
        display_name: &str,
    ) -> Result<(), String> {
        let access = class
            .field_access
            .get(member_key)
            .or_else(|| class.method_access.get(member_key))
            .cloned()
            .unwrap_or(Accessibility::Public);
        match access {
            Accessibility::Public => Ok(()),
            Accessibility::Private => {
                if let Some(cur) = &self.current_class {
                    if cur.name == class.name {
                        return Ok(());
                    }
                }
                Err(format!(
                    "AccessError: '{}' is private and cannot be accessed outside '{}'",
                    display_name, class.name
                ))
            }
            Accessibility::Protected => {
                if let Some(cur) = &self.current_class {
                    if cur.name == class.name {
                        return Ok(());
                    }
                    // subclass: current_class has class.name in its bases
                    if cur.bases.contains(&class.name) {
                        return Ok(());
                    }
                }
                Err(format!(
                    "AccessError: '{}' is protected and cannot be accessed outside '{}' or its subclasses",
                    display_name, class.name
                ))
            }
        }
    }

    /// 式（`Expr`）を評価して `Value` を返す。各バリアントを専用メソッドに委譲する薄いディスパッチャ。
    pub fn eval(&mut self, expr: &Expr) -> Result<Value, String> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(f) => Ok(Value::Float(*f)),
            Expr::ImaginaryLit(f) => Ok(Value::Complex(0.0, *f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::None => Ok(Value::None),
            Expr::Ident(name) => self
                .get_val(name)
                .ok_or_else(|| format!("NameError: '{name}' is not defined")),
            Expr::DebugVar(name) => self
                .dbg_vars
                .get(name)
                .map(|v| v.get_value())
                .ok_or_else(|| format!("NameError: 'dbg::{name}' is not defined")),
            Expr::LocalVar(name) => {
                let key = format!("local::{name}");
                self.get_val(&key).ok_or_else(|| {
                    format!(
                        "NameError: 'local::{name}' is not defined \
                         (only valid inside a function with variadic parameter `...`)"
                    )
                })
            }
            Expr::TraitAccess { object, trait_name, attr } => {
                self.eval_trait_access(object, trait_name, attr)
            }
            Expr::Attr { object, attr, .. } => self.eval_attr(object, attr),
            Expr::List(items) => {
                let mut vals = Vec::new();
                for item in items {
                    vals.push(self.eval(item)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(vals))))
            }
            Expr::Tuple(exprs) => {
                let mut values = Vec::new();
                let mut types = Vec::new();
                for expr in exprs {
                    let val = self.eval(expr)?;
                    types.push(self.type_name(&val).to_string());
                    values.push(val);
                }
                Ok(Value::Tuple(Rc::new(TupleData::new(values, types))))
            }
            Expr::Dict(pairs) => {
                let mut d = DictData::new("Any".to_string(), "Any".to_string());
                for (key_expr, val_expr) in pairs {
                    let k = self.eval(key_expr)?;
                    let v = self.eval(val_expr)?;
                    d.set(k, v);
                }
                Ok(Value::Dict(Rc::new(RefCell::new(d))))
            }
            Expr::Set(items) => {
                let mut vals: Vec<Value> = Vec::new();
                for item in items {
                    let v = self.eval(item)?;
                    set_insert(&mut vals, v, self);
                }
                Ok(Value::Set(Rc::new(RefCell::new(vals))))
            }
            Expr::Subscript { object, index } => {
                let obj = self.eval(object)?;
                let key = self.eval(index)?;
                self.eval_subscript(obj, key)
            }
            Expr::Slice { begin, end, step } => self.eval_slice_expr(begin, end, step),
            Expr::UnaryOp { op, operand } => {
                let val = self.eval(operand)?;
                self.apply_unary_dyn(op, val)
            }
            Expr::BinOp { op, left, right, .. } => self.eval_binop_expr(op, left, right),
            Expr::TemplateInstantiate { .. } => Err(
                "TemplateError: template expression must be immediately called (e.g. `Func[T](args)`)".to_string()
            ),
            Expr::Block { stmts, return_type } => {
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().push(return_type.clone()));
                let result = self.eval_block_expr(stmts);
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().pop());
                result
            }
            Expr::IfExpr { branches, else_body, return_type } => {
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().push(return_type.clone()));
                let result = self.eval_if_expr_body(branches, else_body);
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().pop());
                result
            }
            Expr::ForExpr { target, iter, body, return_type } => {
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().push(return_type.clone()));
                let result = self.eval_for_expr(target, iter, body);
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().pop());
                result
            }
            Expr::WhileExpr { cond, body, return_type } => {
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().push(return_type.clone()));
                let result = self.eval_while_expr(cond, body);
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().pop());
                result
            }
            Expr::MatchExpr { subject, arms, return_type } => {
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().push(return_type.clone()));
                let result = self.eval_match_expr(subject, arms);
                BLOCK_RETURN_EXPECTED_TYPE.with(|t| t.borrow_mut().pop());
                result
            }
            Expr::IsType { expr, negated, type_name, .. } => {
                let val = self.eval(expr)?;
                let result = self.value_is_type(&val, type_name);
                Ok(Value::Bool(if *negated { !result } else { result }))
            }
            Expr::Call { func, args, span } => self.eval_call(func, args, span),
            Expr::Cast { object, type_name, .. } => self.eval_cast(object, type_name),
        }
    }

    // --- eval() から抽出したメソッド群 ---

    /// トレイトアクセス式 `obj:TraitName::attr` を評価する。
    /// インスタンスのフィールドマップから名前空間付きキー `TraitName::attr` を検索して返す。
    fn eval_trait_access(
        &mut self,
        object: &Expr,
        trait_name: &str,
        attr: &str,
    ) -> Result<Value, String> {
        let obj_val = self.eval(object)?;
        match obj_val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let key = format!("{}::{}", trait_name, attr);
                if let Some((v, _)) = inst.fields.get(&key) {
                    return Ok(v.clone());
                }
                Err(format!(
                    "AttributeError: trait field '{trait_name}::{attr}' not found on '{}'",
                    inst.class.name
                ))
            }
            _ => Err("AttributeError: cannot access trait field on non-instance".to_string()),
        }
    }

    /// 属性アクセス式 `obj.attr` を評価する。`get_attr_val` に委譲するシンラッパー。
    fn eval_attr(&mut self, object: &Expr, attr: &str) -> Result<Value, String> {
        let obj_val = self.eval(object)?;
        self.get_attr_val(obj_val, attr)
    }

    /// スライス式 `begin:end:step` を評価して `Value::Slice` を生成する。
    /// 各境界は int・Index インスタンス・None のいずれかでなければならない。
    fn eval_slice_expr(
        &mut self,
        begin: &Option<Box<Expr>>,
        end: &Option<Box<Expr>>,
        step: &Option<Box<Expr>>,
    ) -> Result<Value, String> {
        let begin = match begin {
            None => None,
            Some(e) => {
                let v = self.eval(e)?;
                match &v {
                    Value::None => None,
                    Value::Int(_) => Some(v),
                    Value::Instance(inst) if inst.borrow().class.name == "Index" => Some(v),
                    _ => {
                        return Err(format!(
                            "TypeError: slice begin must be int, Index, or None, got '{}'",
                            self.type_name(&v)
                        ))
                    }
                }
            }
        };
        let end = match end {
            None => None,
            Some(e) => {
                let v = self.eval(e)?;
                match &v {
                    Value::None => None,
                    Value::Int(_) => Some(v),
                    Value::Instance(inst) if inst.borrow().class.name == "Index" => Some(v),
                    _ => {
                        return Err(format!(
                            "TypeError: slice end must be int, Index, or None, got '{}'",
                            self.type_name(&v)
                        ))
                    }
                }
            }
        };
        let step = match step {
            None => None,
            Some(e) => {
                let v = self.eval(e)?;
                match &v {
                    Value::None => None,
                    Value::Int(_) => Some(v),
                    _ => {
                        return Err(format!(
                            "TypeError: slice step must be int or None, got '{}'",
                            self.type_name(&v)
                        ))
                    }
                }
            }
        };
        Ok(Value::Slice(Rc::new(SliceValue { begin, end, step })))
    }

    /// 二項演算式を評価する。`And` / `Or` は短絡評価、それ以外は両辺を評価して `apply_binop` に渡す。
    fn eval_binop_expr(&mut self, op: &BinOp, left: &Expr, right: &Expr) -> Result<Value, String> {
        match op {
            BinOp::And => {
                let lv = self.eval(left)?;
                if !self.eval_truthy(&lv)? {
                    Ok(lv)
                } else {
                    self.eval(right)
                }
            }
            BinOp::Or => {
                let lv = self.eval(left)?;
                if self.eval_truthy(&lv)? {
                    Ok(lv)
                } else {
                    self.eval(right)
                }
            }
            _ => {
                let lv = self.eval(left)?;
                let rv = self.eval(right)?;
                self.apply_binop_dyn(op, lv, rv)
            }
        }
    }

    /// match 式を評価する。各アームのパターンとサブジェクトを照合し、最初に一致したアームのボディを実行して値を返す。
    fn eval_match_expr(&mut self, subject: &Expr, arms: &[MatchArm]) -> Result<Value, String> {
        let subject_val = self.eval(subject)?;
        for arm in arms {
            let matched = match &arm.pattern {
                MatchPattern::Case(pattern_expr) => {
                    if matches!(pattern_expr, Expr::Ident(n) if n == "_") {
                        true
                    } else {
                        let pv = self.eval(pattern_expr)?;
                        matches!(
                            self.apply_binop_dyn(&BinOp::Eq, subject_val.clone(), pv)?,
                            Value::Bool(true)
                        )
                    }
                }
                MatchPattern::IsType(type_name) => self.value_is_type(&subject_val, type_name),
            };
            if matched {
                return self.eval_capture_block_return(&arm.body);
            }
        }
        Ok(Value::None)
    }

    /// 関数呼び出し式 `func(args)` を評価する。
    /// テンプレート instantiate・メソッド呼び出し・組み込み関数・ユーザー定義関数・クラスコンストラクタ・
    /// ジェネレータ・ネイティブ関数・型コンストラクタなど、呼び出し先の種別に応じて適切なパスへ分岐する。
    fn eval_call(&mut self, func: &Expr, args: &[CallArg], call_span: &Span) -> Result<Value, String> {
        if let Expr::TemplateInstantiate { base, type_args } = func {
            let tmpl_val = self.eval(base)?;
            return self.instantiate_template(tmpl_val, type_args, args);
        }
        if let Expr::Attr { object, attr, .. } = func {
            let obj_val = self.eval(object)?;
            return self.eval_method_call(obj_val, attr, args);
        }
        if let Expr::Ident(name) = func {
            if let Some(result) = self.eval_builtin_ident_call(name, args) {
                return result;
            }
        }
        let call_name = match func {
            Expr::Ident(n) => n.clone(),
            _ => "<anonymous>".to_string(),
        };
        let callee = self.eval(func)?;
        match callee {
            Value::Function(fn_val) => self.exec_fn(fn_val, args, None, &call_name, Some(call_span.clone())),
            Value::OverloadedFn(candidates) => {
                let evaled_args = self.eval_call_args(args)?;
                self.dispatch_overload_evaled(candidates, evaled_args, None, &call_name, Some(call_span.clone()))
            }
            Value::Class(cls) => self.instantiate(cls, args),
            Value::GeneratorFn(gen_fn) => self.exec_generator(gen_fn, args, None),
            Value::TemplateFn(_) | Value::TemplateClass(_) | Value::TemplateGenFn(_) => Err(
                "TemplateError: template must be called with explicit type arguments (e.g. `Func[T](args)`)".to_string()
            ),
            Value::PyObject(handle) => {
                let evaled_args = self.eval_call_args(args)?;
                super::py_interop::call_py_object(&handle, &evaled_args)
            }
            Value::Instance(_) => {
                self.eval_method_call(callee, "__call__", args)
            }
            Value::NativeFunction(fn_ref) => {
                self.call_native_function(&fn_ref, args)
            }
            Value::Type(type_name) => {
                self.eval_type_constructor_call(&type_name, args)
            }
            other => Err(format!("TypeError: '{}' object is not callable", self.type_name(&other))),
        }
    }

    /// キャスト式 `obj => TypeName` を評価する。
    ///
    /// 動作:
    /// 1. ターゲット型が `new_type` クラスの場合 → コンストラクタ呼び出し `TypeName(inner_val)`
    ///    (obj 自身が new_type インスタンスのときは先に `.value` を取り出してからラップする)
    /// 2. obj が new_type インスタンスかつターゲット型がそのベース型の場合 → `.value` を返す
    /// 3. オブジェクトがインスタンスで `__cast__[TypeName]` メソッドを持つ場合 → そのメソッドを呼び出す
    /// 4. それ以外 → TypeError
    fn eval_cast(&mut self, object: &crate::ast::Expr, type_name: &str) -> Result<Value, String> {
        let obj = self.eval(object)?;

        // new_type インスタンスなら内部値を先に取り出しておく
        let inner_val = if let Value::Instance(ref inst_rc) = obj {
            let cls = inst_rc.borrow().class.clone();
            if cls.new_type_base.is_some() {
                inst_rc.borrow().fields.get("value").map(|(v, _)| v.clone())
            } else {
                None
            }
        } else {
            None
        };

        // --- new_type へのダウンキャスト: TypeName(obj) と等価 ---
        // obj 自身が new_type インスタンスの場合は内部値を渡してネストを防ぐ
        if let Some(target_val) = self.get_val(type_name) {
            if let Value::Class(ref cls) = target_val {
                if cls.new_type_base.is_some() {
                    let cls_rc = cls.clone();
                    let arg = inner_val.unwrap_or(obj);
                    return self.instantiate_evaled(cls_rc, vec![(None, arg)]);
                }
            }
        }

        // --- list ⇒ fixed_list: flat conversion ---
        let target_is_fixed = type_name == "fixed_list"
            || type_name.starts_with("fixed_list[");
        let target_is_list = type_name == "list"
            || type_name.starts_with("list[");
        if target_is_fixed {
            return match obj {
                Value::FrozenList { .. } => Ok(obj),  // already a fixed_list
                Value::List(ref rc) => {
                    let items = rc.borrow().clone();
                    Self::try_flat_freeze(&items).ok_or_else(|| {
                        "CastError: cannot cast list to fixed_list: \
                         elements must be homogeneous class instances \
                         with only int/float fields".to_string()
                    })
                }
                _ => Err(format!(
                    "CastError: cannot cast '{}' to 'fixed_list'",
                    self.type_name(&obj)
                )),
            };
        }
        if target_is_list {
            if let Value::FrozenList { ref state, ref layout } = obj {
                let st = state.borrow();
                let items = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                return Ok(Value::List(Rc::new(RefCell::new(items))));
            }
        }

        // --- インスタンスの __cast__[TypeName] メソッド呼び出し ---
        match &obj {
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();

                // new_type インスタンスをそのベース型にキャスト: .value を返す
                if let Some(ref base) = class.new_type_base {
                    if base == type_name {
                        let val = inst_rc.borrow().fields.get("value").map(|(v, _)| v.clone());
                        return val.ok_or_else(|| {
                            format!("TypeError: '{}' has no 'value' field", class.name)
                        });
                    }
                }

                let method_key = format!("__cast__[{}]", type_name);
                let overloads = self
                    .lookup_method_in_class(&class, &method_key)
                    .ok_or_else(|| {
                        format!(
                        "TypeError: '{}' is not castable to '{}' (no __cast__[{}] method defined)",
                        class.name, type_name, type_name
                    )
                    })?;
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(obj), "__cast__", None)
                } else {
                    self.dispatch_overload(overloads, &[], Some(obj), None)
                }
            }
            other => Err(format!(
                "TypeError: cast operator '=>' requires an instance or new_type target, \
                 got '{}' cast to '{}'",
                self.type_name(other),
                type_name
            )),
        }
    }

    /// 組み込み関数名を受け取り、該当する組み込みを実行して結果を返す。
    /// 未知の名前には `None` を返してユーザー定義関数の探索にフォールスルーする。
    fn eval_builtin_ident_call(
        &mut self,
        name: &str,
        args: &[CallArg],
    ) -> Option<Result<Value, String>> {
        match name {
            "print" => {
                let mut parts: Vec<String> = Vec::new();
                for a in args {
                    let v = match self.eval(a.expr()) {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    };
                    let s = match self.display_str(&v) {
                        Ok(s) => s,
                        Err(e) => return Some(Err(e)),
                    };
                    parts.push(s);
                }
                println!("{}", parts.join(" "));
                Some(Ok(Value::None))
            }
            "next" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: next() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match val {
                    v @ Value::Generator(_) => self.eval_method_call_evaled(v, "next", vec![]),
                    v @ Value::Instance(_) => self.eval_method_call_evaled(v, "__next__", vec![]),
                    other => Err(format!(
                        "TypeError: '{}' object is not an iterator",
                        self.type_name(&other)
                    )),
                })
            }
            "repr" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: repr() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(self.repr_val(&val).map(Value::Str))
            }
            "range" => {
                let evaled: Result<Vec<_>, _> = args.iter().map(|a| self.eval(a.expr())).collect();
                let evaled = match evaled {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match evaled.as_slice() {
                    [Value::Int(stop)] => Ok(Value::List(Rc::new(RefCell::new(
                        (0..*stop).map(Value::Int).collect(),
                    )))),
                    [Value::Int(start), Value::Int(stop)] => Ok(Value::List(Rc::new(
                        RefCell::new((*start..*stop).map(Value::Int).collect()),
                    ))),
                    [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                        let mut items = Vec::new();
                        let mut i = *start;
                        if *step > 0 {
                            while i < *stop {
                                items.push(Value::Int(i));
                                i += step;
                            }
                        } else if *step < 0 {
                            while i > *stop {
                                items.push(Value::Int(i));
                                i += step;
                            }
                        }
                        Ok(Value::List(Rc::new(RefCell::new(items))))
                    }
                    _ => Err("TypeError: range() takes 1\u{2013}3 integer arguments".to_string()),
                })
            }
            "len" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: len() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                // Instance の __len__ を優先チェック（borrow を落としてから呼び出す）
                let has_instance_len = if let Value::Instance(inst_rc) = &val {
                    inst_rc.borrow().class.methods.contains_key("__len__")
                } else {
                    false
                };
                if has_instance_len {
                    return Some(
                        self.eval_method_call_evaled(val, "__len__", vec![])
                            .and_then(|r| match r {
                                Value::Int(n) => Ok(Value::Int(n)),
                                other => Err(format!(
                                    "TypeError: __len__ must return int, not '{}'",
                                    self.type_name(&other)
                                )),
                            }),
                    );
                }
                Some(match &val {
                    Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
                    Value::FrozenList { ref state, .. } => Ok(Value::Int(state.borrow().len as i64)),
                    Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                    Value::Dict(d) => Ok(Value::Int(d.borrow().all_keys().len() as i64)),
                    Value::Set(s) => Ok(Value::Int(s.borrow().len() as i64)),
                    Value::Tuple(t) => Ok(Value::Int(t.len() as i64)),
                    Value::PyObject(handle) => super::py_interop::py_len(handle),
                    _ => Err(format!(
                        "TypeError: object of type '{}' has no len()",
                        self.type_name(&val)
                    )),
                })
            }
            // ── mutable flat-list built-ins ───────────────────────────────────
            // create_flat_int_list(size, val) → fixed_list[Cell]
            // Allocates a flat byte buffer directly — no Cell instance allocation.
            // Requires 'Cell' class to be in scope (via `from ant_render import Cell`).
            "create_flat_int_list" => {
                if args.len() != 2 {
                    return Some(Err("TypeError: create_flat_int_list() takes exactly 2 arguments".to_string()));
                }
                let size_v = match self.eval(args[0].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let init_v = match self.eval(args[1].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let size = match &size_v { Value::Int(n) => *n as usize, _ => return Some(Err("TypeError: create_flat_int_list: size must be int".to_string())) };
                let init = match &init_v { Value::Int(n) => *n, _ => return Some(Err("TypeError: create_flat_int_list: val must be int".to_string())) };
                let cell_class = match self.get_val("Cell") {
                    Some(Value::Class(c)) => c,
                    _ => return Some(Err("NameError: create_flat_int_list requires 'Cell' class in scope".to_string())),
                };
                let init_bytes = init.to_le_bytes();
                let mut raw = vec![0u8; size * 8];
                for chunk in raw.chunks_exact_mut(8) { chunk.copy_from_slice(&init_bytes); }
                let flat_data = super::value::FlatListData { data: raw, len: size, allocated_size: size };
                let layout = super::value::FlatLayout {
                    class_name: "Cell".to_string(),
                    fields: vec![("v".to_string(), super::value::FlatFieldTy::Int)],
                    stride: 8,
                    class: cell_class,
                };
                Some(Ok(Value::FrozenList { state: Rc::new(RefCell::new(flat_data)), layout: Rc::new(layout) }))
            }
            // flat_get_int(grid, idx) → int
            "flat_get_int" => {
                if args.len() != 2 {
                    return Some(Err("TypeError: flat_get_int() takes exactly 2 arguments".to_string()));
                }
                let grid_v = match self.eval(args[0].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx_v  = match self.eval(args[1].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx = match &idx_v { Value::Int(n) => *n as usize, _ => return Some(Err("TypeError: flat_get_int: idx must be int".to_string())) };
                match &grid_v {
                    Value::FrozenList { state, .. } => {
                        let s = state.borrow();
                        if idx >= s.len { return Some(Err(format!("IndexError: flat_get_int index {idx} out of range (len {})", s.len))); }
                        let off = idx * 8;
                        let bytes: [u8; 8] = s.data[off..off + 8].try_into().unwrap();
                        Some(Ok(Value::Int(i64::from_le_bytes(bytes))))
                    }
                    _ => Some(Err(format!("TypeError: flat_get_int expects fixed_list, got {}", self.type_name(&grid_v)))),
                }
            }
            // flat_set_int(grid, idx, val) → None  — writes directly into the flat buffer
            "flat_set_int" => {
                if args.len() != 3 {
                    return Some(Err("TypeError: flat_set_int() takes exactly 3 arguments".to_string()));
                }
                let grid_v = match self.eval(args[0].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx_v  = match self.eval(args[1].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let val_v  = match self.eval(args[2].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx = match &idx_v { Value::Int(n) => *n as usize, _ => return Some(Err("TypeError: flat_set_int: idx must be int".to_string())) };
                let val = match &val_v { Value::Int(n) => *n, _ => return Some(Err("TypeError: flat_set_int: val must be int".to_string())) };
                match &grid_v {
                    Value::FrozenList { state, .. } => {
                        let mut s = state.borrow_mut();
                        if idx >= s.len { return Some(Err(format!("IndexError: flat_set_int index {idx} out of range"))); }
                        let off = idx * 8;
                        s.data[off..off + 8].copy_from_slice(&val.to_le_bytes());
                        Some(Ok(Value::None))
                    }
                    _ => Some(Err(format!("TypeError: flat_set_int expects fixed_list, got {}", self.type_name(&grid_v)))),
                }
            }
            // ─────────────────────────────────────────────────────────────────
            "id" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: id() takes exactly one argument".to_string()));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(self.call_type_by_name_evaled("id", vec![val]))
            }
            "open" => Some(self.eval_builtin_open(args)),
            "close" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: close() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match val {
                    Value::FileObject(fd_rc) => {
                        fd_rc.borrow_mut().close();
                        Ok(Value::None)
                    }
                    other => Err(format!(
                        "TypeError: close() argument must be FileObject, not '{}'",
                        self.type_name(&other)
                    )),
                })
            }
            "enumerate" => {
                let mut positional: Vec<Value> = Vec::new();
                let mut start_val: Option<Value> = None;
                for arg in args {
                    match arg {
                        CallArg::Positional(e) => match self.eval(e) {
                            Ok(v) => positional.push(v),
                            Err(e) => return Some(Err(e)),
                        },
                        CallArg::Keyword { name, value } if name == "start" => {
                            match self.eval(value) {
                                Ok(v) => start_val = Some(v),
                                Err(e) => return Some(Err(e)),
                            }
                        }
                        CallArg::Keyword { name, .. } => {
                            return Some(Err(format!(
                                "TypeError: enumerate() got unexpected keyword argument '{name}'"
                            )));
                        }
                        CallArg::Variadic(_) => {
                            return Some(Err(
                                "TypeError: enumerate() does not support variadic arguments".to_string()
                            ));
                        }
                    }
                }
                if positional.len() != 1 {
                    return Some(Err(format!(
                        "TypeError: enumerate() expected 1 positional argument, got {}",
                        positional.len()
                    )));
                }
                let start = match start_val {
                    Some(Value::Int(n)) => n,
                    Some(other) => {
                        return Some(Err(format!(
                            "TypeError: enumerate() 'start' must be int, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    None => 0i64,
                };
                let items = match self.collect_iterable(positional.into_iter().next().unwrap()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                let tuples: Vec<Value> = items
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let idx = start + i as i64;
                        let type_str = self.type_name(&v).to_string();
                        Value::Tuple(Rc::new(TupleData::new(
                            vec![Value::Int(idx), v],
                            vec!["int".to_string(), type_str],
                        )))
                    })
                    .collect();
                Some(Ok(Value::Generator(Rc::new(RefCell::new(
                    GeneratorState {
                        values: tuples,
                        index: 0,
                    },
                )))))
            }
            "zip" => {
                for arg in args.iter() {
                    if matches!(arg, CallArg::Keyword { .. }) {
                        return Some(Err(
                            "TypeError: zip() takes no keyword arguments".to_string()
                        ));
                    }
                }
                let mut iters: Vec<Vec<Value>> = Vec::new();
                for arg in args.iter() {
                    let v = match self.eval(arg.expr()) {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    };
                    let items = match self.collect_iterable(v) {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    };
                    iters.push(items);
                }
                if iters.is_empty() {
                    return Some(Ok(Value::Generator(Rc::new(RefCell::new(
                        GeneratorState {
                            values: vec![],
                            index: 0,
                        },
                    )))));
                }
                let min_len = iters.iter().map(|it| it.len()).min().unwrap_or(0);
                let tuples: Vec<Value> = (0..min_len)
                    .map(|i| {
                        let vals: Vec<Value> = iters.iter().map(|it| it[i].clone()).collect();
                        let types: Vec<String> =
                            vals.iter().map(|v| self.type_name(v).to_string()).collect();
                        Value::Tuple(Rc::new(TupleData::new(vals, types)))
                    })
                    .collect();
                Some(Ok(Value::Generator(Rc::new(RefCell::new(
                    GeneratorState {
                        values: tuples,
                        index: 0,
                    },
                )))))
            }
            "getenv" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(
                        "TypeError: getenv() takes 1 or 2 arguments (name[, default])".to_string(),
                    ));
                }
                let name = match self.eval(args[0].expr()) {
                    Ok(Value::Str(s)) => s,
                    Ok(other) => {
                        return Some(Err(format!(
                            "TypeError: getenv() name must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    Err(e) => return Some(Err(e)),
                };
                let default = if args.len() > 1 {
                    match self.eval(args[1].expr()) {
                        Ok(Value::Str(s)) => s,
                        Ok(other) => {
                            return Some(Err(format!(
                                "TypeError: getenv() default must be str, not '{}'",
                                self.type_name(&other)
                            )))
                        }
                        Err(e) => return Some(Err(e)),
                    }
                } else {
                    String::new()
                };
                Some(Ok(Value::Str(std::env::var(&name).unwrap_or(default))))
            }
            "parse_ar" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(
                        "TypeError: parse_ar() takes 1 or 2 arguments (source[, path])".to_string(),
                    ));
                }
                let source = match self.eval(args[0].expr()) {
                    Ok(Value::Str(s)) => s,
                    Ok(other) => {
                        return Some(Err(format!(
                            "TypeError: parse_ar() source must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    Err(e) => return Some(Err(e)),
                };
                let path = if args.len() > 1 {
                    match self.eval(args[1].expr()) {
                        Ok(Value::Str(s)) => s,
                        Ok(other) => {
                            return Some(Err(format!(
                                "TypeError: parse_ar() path must be str, not '{}'",
                                self.type_name(&other)
                            )))
                        }
                        Err(e) => return Some(Err(e)),
                    }
                } else {
                    String::new()
                };
                let source = source.strip_prefix('\u{FEFF}').map(str::to_string).unwrap_or(source);
                let tokens = crate::lexer::Lexer::new(&source, &*path).tokenize();
                let source_dir = std::path::Path::new(&path)
                    .parent()
                    .map(|p| p.to_path_buf());
                let stmts = match crate::parser::Parser::new(tokens, source_dir).parse_program() {
                    Ok(s) => s,
                    Err(e) => return Some(Err(format!("ParseError in parse_ar: {e}"))),
                };
                Some(Ok(super::ast_value::stmts_to_value(&stmts)))
            }
            _ => None,
        }
    }

    /// 組み込み関数 `open()` を実行してファイルオブジェクト (`Value::FileObject`) を返す。
    /// 引数 `file_path`, `open_mode`, `start_point`, `byte_recognizing`, `encoding`, `exclusion` を解析し、
    /// 対応する `std::fs::OpenOptions` を構築してファイルを開く。
    fn eval_builtin_open(&mut self, args: &[CallArg]) -> Result<Value, String> {
        use std::collections::HashMap as HMap;
        use std::fs::OpenOptions;
        use std::io::Read as IoRead;
        let evaled = self.eval_call_args(args)?;
        let mut kw: HMap<String, Value> = HMap::new();
        let mut pos: Vec<Value> = Vec::new();
        for (k, v) in evaled {
            match k {
                Some(n) => {
                    kw.insert(n, v);
                }
                None => pos.push(v),
            }
        }
        let file_path = extract_path_str(
            get_arg(&pos, &kw, 0, "file_path")
                .ok_or("TypeError: open() missing required argument 'file_path'")?,
        )?;
        let open_mode_int = extract_enum_int(
            get_arg(&pos, &kw, 1, "open_mode")
                .ok_or("TypeError: open() missing required argument 'open_mode'")?,
            "enum_item_FileOpenMode",
        )?;
        let start_point_int: i64 = get_arg(&pos, &kw, 2, "start_point")
            .map(|v| extract_enum_int(v, "enum_item_StartPoint"))
            .transpose()?
            .unwrap_or(0);
        let byte_mode_int: i64 = get_arg(&pos, &kw, 3, "byte_recognizing")
            .map(|v| extract_enum_int(v, "enum_item_ByteRecognizingMode"))
            .transpose()?
            .unwrap_or(1);
        let enc_int: i64 = get_arg(&pos, &kw, 4, "encoding")
            .map(|v| extract_enum_int(v, "enum_item_Encoding"))
            .transpose()?
            .unwrap_or(1);
        if enc_int == 3 {
            return Err("NotImplementedError: Shift-JIS encoding is not yet supported".to_string());
        }
        let _exclusion: bool = get_arg(&pos, &kw, 5, "exclusion")
            .map(|v| match v {
                Value::Bool(b) => Ok(*b),
                _ => Err("TypeError: open() 'exclusion' must be bool".to_string()),
            })
            .transpose()?
            .unwrap_or(true);

        let mode = match open_mode_int {
            0 => FileOpenModeRust::Write,
            1 => FileOpenModeRust::Rewrite,
            2 => FileOpenModeRust::Read,
            3 => FileOpenModeRust::MakeAndWrite,
            n => return Err(format!("TypeError: invalid FileOpenMode value {n}")),
        };
        let byte_mode = if byte_mode_int == 0 {
            ByteModeRust::Byte
        } else {
            ByteModeRust::Text
        };

        let std_path = std::path::Path::new(&file_path);
        if mode == FileOpenModeRust::MakeAndWrite && std_path.exists() {
            return Err(format!(
                "RuntimeError: open() make_and_write: file '{}' already exists",
                file_path
            ));
        }

        let (file, content) = match mode {
            FileOpenModeRust::Read => {
                let mut f = OpenOptions::new()
                    .read(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                let mut c = Vec::new();
                f.read_to_end(&mut c)
                    .map_err(|e| format!("IOError: cannot read '{}': {e}", file_path))?;
                (f, c)
            }
            FileOpenModeRust::Write => {
                let mut f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                let mut c = Vec::new();
                f.read_to_end(&mut c)
                    .map_err(|e| format!("IOError: cannot read '{}': {e}", file_path))?;
                (f, c)
            }
            FileOpenModeRust::Rewrite => {
                let f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                (f, Vec::new())
            }
            FileOpenModeRust::MakeAndWrite => {
                let f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot create '{}': {e}", file_path))?;
                (f, Vec::new())
            }
        };

        let (content, bom_skip) = if enc_int == 2 && content.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (content[3..].to_vec(), 3usize)
        } else {
            (content, 0usize)
        };
        let _ = bom_skip;
        let pointer = if start_point_int == 1 {
            content.len()
        } else {
            0
        };

        let fd = FileData {
            path: file_path,
            mode,
            byte_mode,
            content,
            pointer,
            is_closed: false,
            file_handle: Some(file),
        };
        Ok(Value::FileObject(Rc::new(RefCell::new(fd))))
    }

    /// 型コンストラクタ呼び出し（`list(...)`, `dict(...)`, `str(...)` 等）を評価する。
    fn eval_type_constructor_call(
        &mut self,
        type_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        if type_name == "AsyncManager" {
            return self.make_async_manager(args);
        }
        // Signal[T]() — Arrow ネイティブのイベントソースを生成する。
        // テンプレート引数 T はランタイムでは無視する。
        if type_name == "Signal" {
            return Ok(Value::Signal(std::rc::Rc::new(std::cell::RefCell::new(
                super::event_loop::SignalData::new(),
            ))));
        }
        let evaled = self.eval_call_args(args)?;
        let vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
        self.call_type_by_name_evaled(type_name, vals)
    }

    /// Dispatch an already-evaluated argument list to a built-in type constructor.
    /// Called by `eval_type_constructor_call` (from AST) and `call_value_with_args`
    /// (from native callbacks that hold a `Value::Type`).
    pub(super) fn call_type_by_name_evaled(
        &mut self,
        type_name: &str,
        vals: Vec<Value>,
    ) -> Result<Value, String> {
        match type_name {
            "str" => {
                let has_instance_str = if let [Value::Instance(inst_rc)] = vals.as_slice() {
                    inst_rc.borrow().class.methods.contains_key("__str__")
                } else {
                    false
                };
                if has_instance_str {
                    let v = vals.into_iter().next().unwrap();
                    return self.eval_method_call_evaled(v, "__str__", vec![])
                        .and_then(|r| match r {
                            Value::Str(s) => Ok(Value::Str(s)),
                            other => Ok(Value::Str(self.display(&other))),
                        });
                }
                match vals.as_slice() {
                    [] => Ok(Value::Str(String::new())),
                    [v] => Ok(Value::Str(self.display(v))),
                    _ => Err("TypeError: str() takes at most 1 argument".to_string()),
                }
            },
            "int" => match vals.as_slice() {
                [] => Ok(Value::Int(0)),
                [Value::Int(n)] => Ok(Value::Int(*n)),
                [Value::Float(f)] => Ok(Value::Int(*f as i64)),
                [Value::Bool(b)] => Ok(Value::Int(if *b { 1 } else { 0 })),
                [Value::Str(s)] => s
                    .trim()
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| format!("ValueError: invalid literal for int(): '{s}'")),
                [other] => Err(format!(
                    "TypeError: int() argument must be a string or a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: int() takes at most 1 argument".to_string()),
            },
            "float" => match vals.as_slice() {
                [] => Ok(Value::Float(0.0)),
                [Value::Float(f)] => Ok(Value::Float(*f)),
                [Value::Int(n)] => Ok(Value::Float(*n as f64)),
                [Value::Bool(b)] => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
                [Value::Str(s)] => s
                    .trim()
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| format!("ValueError: invalid literal for float(): '{s}'")),
                [other] => Err(format!(
                    "TypeError: float() argument must be a string or a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: float() takes at most 1 argument".to_string()),
            },
            "complex" => match vals.as_slice() {
                [] => Ok(Value::Complex(0.0, 0.0)),
                [Value::Complex(re, im)] => Ok(Value::Complex(*re, *im)),
                [Value::Float(f)] => Ok(Value::Complex(*f, 0.0)),
                [Value::Int(n)] => Ok(Value::Complex(*n as f64, 0.0)),
                [Value::Complex(re, _), Value::Float(im2)] => Ok(Value::Complex(*re, *im2)),
                [Value::Float(re), Value::Float(im)] => Ok(Value::Complex(*re, *im)),
                [Value::Int(re), Value::Int(im)] => Ok(Value::Complex(*re as f64, *im as f64)),
                [Value::Int(re), Value::Float(im)] => Ok(Value::Complex(*re as f64, *im)),
                [Value::Float(re), Value::Int(im)] => Ok(Value::Complex(*re, *im as f64)),
                [other] => Err(format!(
                    "TypeError: complex() argument must be a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: complex() takes at most 2 arguments".to_string()),
            },
            "bool" => match vals.as_slice() {
                [] => Ok(Value::Bool(false)),
                [Value::Bool(b)] => Ok(Value::Bool(*b)),
                [Value::Int(n)] => Ok(Value::Bool(*n != 0)),
                [Value::Float(f)] => Ok(Value::Bool(*f != 0.0)),
                [Value::Str(s)] => Ok(Value::Bool(!s.is_empty())),
                [Value::None] => Ok(Value::Bool(false)),
                [Value::List(lst)] => Ok(Value::Bool(!lst.borrow().is_empty())),
                [Value::Set(s)] => Ok(Value::Bool(!s.borrow().is_empty())),
                [_] => Ok(Value::Bool(true)),
                _ => Err("TypeError: bool() takes at most 1 argument".to_string()),
            },
            "list" => match vals {
                ref v if v.is_empty() => Ok(Value::List(Rc::new(RefCell::new(vec![])))),
                _ if vals.len() == 1 => match vals.into_iter().next().unwrap() {
                    Value::List(lst) => Ok(Value::List(lst)),
                    Value::FrozenList { state, layout } => {
                        let st = state.borrow();
                        let items = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                        Ok(Value::List(Rc::new(RefCell::new(items))))
                    }
                    Value::Set(s) => Ok(Value::List(Rc::new(RefCell::new(s.borrow().clone())))),
                    Value::Str(s) => {
                        let chars = s.chars().map(|c| Value::Str(c.to_string())).collect();
                        Ok(Value::List(Rc::new(RefCell::new(chars))))
                    }
                    other => Err(format!(
                        "TypeError: '{}' object is not iterable",
                        self.type_name(&other)
                    )),
                },
                _ => Err("TypeError: list() takes at most 1 argument".to_string()),
            },
            "set" => match vals {
                ref v if v.is_empty() => Ok(Value::Set(Rc::new(RefCell::new(vec![])))),
                _ if vals.len() == 1 => {
                    let arg = vals.into_iter().next().unwrap();
                    let items: Vec<Value> = match arg {
                        Value::Set(s) => s.borrow().clone(),
                        Value::List(lst) => lst.borrow().clone(),
                        Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                        Value::Tuple(t) => t.all_values().to_vec(),
                        other => {
                            return Err(format!(
                                "TypeError: '{}' object is not iterable",
                                self.type_name(&other)
                            ))
                        }
                    };
                    let mut result: Vec<Value> = Vec::new();
                    for v in items {
                        set_insert(&mut result, v, self);
                    }
                    Ok(Value::Set(Rc::new(RefCell::new(result))))
                }
                _ => Err("TypeError: set() takes at most 1 argument".to_string()),
            },
            "slice" => {
                let check_index = |v: Value, label: &str| -> Result<Option<Value>, String> {
                    match v {
                        Value::None => Ok(None),
                        Value::Int(_) => Ok(Some(v)),
                        Value::Instance(ref inst) if inst.borrow().class.name == "Index" => {
                            Ok(Some(v))
                        }
                        other => Err(format!(
                            "TypeError: slice {label} must be int, Index, or None, got '{}'",
                            self.type_name(&other)
                        )),
                    }
                };
                let check_step = |v: Value| -> Result<Option<Value>, String> {
                    match v {
                        Value::None => Ok(None),
                        Value::Int(_) => Ok(Some(v)),
                        other => Err(format!(
                            "TypeError: slice step must be int or None, got '{}'",
                            self.type_name(&other)
                        )),
                    }
                };
                match vals.len() {
                    2 => {
                        let mut it = vals.into_iter();
                        let begin = check_index(it.next().unwrap(), "begin")?;
                        let end = check_index(it.next().unwrap(), "end")?;
                        Ok(Value::Slice(Rc::new(SliceValue {
                            begin,
                            end,
                            step: None,
                        })))
                    }
                    3 => {
                        let mut it = vals.into_iter();
                        let begin = check_index(it.next().unwrap(), "begin")?;
                        let end = check_index(it.next().unwrap(), "end")?;
                        let step = check_step(it.next().unwrap())?;
                        Ok(Value::Slice(Rc::new(SliceValue { begin, end, step })))
                    }
                    _ => Err("TypeError: slice() takes 2 or 3 arguments".to_string()),
                }
            }
            "uint" => match vals.as_slice() {
                [] => Ok(Value::UInt(0)),
                [Value::UInt(n)] => Ok(Value::UInt(*n)),
                [Value::Int(n)] => Ok(Value::UInt(*n as u64)),
                [Value::Bool(b)] => Ok(Value::UInt(if *b { 1 } else { 0 })),
                [other] => Err(format!(
                    "TypeError: uint() argument must be an integer, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: uint() takes at most 1 argument".to_string()),
            },
            "id" => {
                if vals.len() != 1 {
                    return Err("TypeError: id() takes exactly one argument".to_string());
                }
                let val = vals.into_iter().next().unwrap();
                let raw: u64 = match &val {
                    Value::Instance(rc) => Rc::as_ptr(rc) as u64,
                    Value::List(rc) => Rc::as_ptr(rc) as u64,
                    Value::Dict(rc) => Rc::as_ptr(rc) as u64,
                    Value::Function(rc) => Rc::as_ptr(rc) as u64,
                    Value::OverloadedFn(v) => v.as_ptr() as u64,
                    Value::Generator(rc) => Rc::as_ptr(rc) as u64,
                    Value::GeneratorFn(rc) => Rc::as_ptr(rc) as u64,
                    Value::Tuple(rc) => Rc::as_ptr(rc) as u64,
                    Value::Int(n) => *n as u64,
                    Value::UInt(n) => *n,
                    Value::Float(f) => f.to_bits(),
                    Value::Bool(b) => *b as u64,
                    Value::Str(s) => {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        s.hash(&mut h);
                        h.finish()
                    }
                    Value::None => 0u64,
                    _ => 0u64,
                };
                let pointer_cls = match self.get_val("pointer") {
                    Some(Value::Class(cls)) => cls,
                    _ => return Err("RuntimeError: 'pointer' type is not defined".to_string()),
                };
                let mut fields = std::collections::HashMap::new();
                fields.insert("value".to_string(), (Value::UInt(raw), true));
                Ok(Value::Instance(Rc::new(RefCell::new(
                    crate::interpreter::InstanceData {
                        class: pointer_cls,
                        fields,
                        immutable: false,
                    },
                ))))
            }
            "len" => {
                let has_instance_len = if let [Value::Instance(inst_rc)] = vals.as_slice() {
                    inst_rc.borrow().class.methods.contains_key("__len__")
                } else {
                    false
                };
                if has_instance_len {
                    let v = vals.into_iter().next().unwrap();
                    return self.eval_method_call_evaled(v, "__len__", vec![])
                        .and_then(|r| match r {
                            Value::Int(n) => Ok(Value::Int(n)),
                            other => Err(format!(
                                "TypeError: __len__ must return int, not '{}'",
                                self.type_name(&other)
                            )),
                        });
                }
                match vals.as_slice() {
                    [Value::List(lst)] => Ok(Value::Int(lst.borrow().len() as i64)),
                    [Value::FrozenList { state, .. }] => Ok(Value::Int(state.borrow().len as i64)),
                    [Value::Str(s)] => Ok(Value::Int(s.len() as i64)),
                    [Value::Dict(d)] => Ok(Value::Int(d.borrow().len() as i64)),
                    [Value::Set(s)] => Ok(Value::Int(s.borrow().len() as i64)),
                    [Value::Tuple(t)] => Ok(Value::Int(t.len() as i64)),
                    [other] => Err(format!(
                        "TypeError: object of type '{}' has no len()",
                        self.type_name(other)
                    )),
                    _ => Err("TypeError: len() takes exactly 1 argument".to_string()),
                }
            },
            other => Err(format!("TypeError: '{}' object is not callable", other)),
        }
    }

    // --- ネイティブ関数呼び出し ---

    /// `Value::NativeFunction` を呼び出す（ハンドルベース ABI）。
    ///
    /// 全引数をバリューアリーナのハンドルに変換し、C ABI ラッパーを呼ぶ。
    /// 結果ハンドルをアリーナから取り出して返す。
    /// `enter_native_call` / `exit_native_call` でアリーナのセーブポイントを管理し、
    /// 呼び出しツリーが終わると一括クリーンアップする。
    pub(super) fn call_native_function(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        args: &[crate::ast::CallArg],
    ) -> Result<Value, String> {
        use crate::interpreter::PtrParam;

        // Fast path: no write-back parameters
        let has_writeback = fn_ref.ptr_params.iter().any(|p| *p == PtrParam::MutPtr);
        if !has_writeback {
            let evaled = self.eval_call_args(args)?;
            let vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
            return self.dispatch_native_evaled(fn_ref, vals);
        }

        // Write-back path — verify MutPtr args that are named variables are mutable.
        // Non-variable expressions (literals, calls, etc.) are allowed but receive no write-back.
        for (i, pp) in fn_ref.ptr_params.iter().enumerate() {
            if *pp != PtrParam::MutPtr {
                continue;
            }
            if let Some(crate::ast::Expr::Ident(name)) = args.get(i).map(|a| a.expr()) {
                let is_mut = self.get_var(name).map(|v| v.is_mutable()).unwrap_or(false);
                if !is_mut {
                    return Err(format!(
                        "TypeError: pointer parameter {i} requires a `mut` variable, '{}' is not mutable",
                        name
                    ));
                }
            }
        }

        let evaled = self.eval_call_args(args)?;
        let mut vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();

        if vals.len() < fn_ref.min_params || vals.len() > fn_ref.n_params {
            let expected = if fn_ref.min_params == fn_ref.n_params {
                format!("{}", fn_ref.n_params)
            } else {
                format!("{}–{}", fn_ref.min_params, fn_ref.n_params)
            };
            return Err(format!(
                "TypeError: native function '{}' expects {} argument(s), got {}",
                fn_ref.fn_name,
                expected,
                vals.len()
            ));
        }
        // Pad missing optional args with None (handle 0 → NULL for pointer params)
        while vals.len() < fn_ref.n_params {
            vals.push(Value::None);
        }

        let is_outermost = super::native_api::enter_native_call(self as *mut Interpreter);

        // Push handles; MutPtr params get writable arena slots
        let mut writebacks: Vec<(String, i64)> = Vec::new();
        let handles: Vec<i64> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let pp = fn_ref.ptr_params.get(i).copied().unwrap_or(PtrParam::None);
                if pp == PtrParam::MutPtr {
                    // Only do write-back when the argument is a named mut variable.
                    // Literals / expressions are passed read-only (no write-back needed).
                    if let Some(crate::ast::Expr::Ident(n)) = args.get(i).map(|a| a.expr()) {
                        let h = super::native_api::push_handle_writeback(v.clone());
                        writebacks.push((n.clone(), h));
                        h
                    } else {
                        super::native_api::push_handle(v.clone())
                    }
                } else {
                    let is_mut = fn_ref.param_mutabilities.get(i).copied().unwrap_or(true);
                    let owned = if is_mut {
                        v.clone()
                    } else {
                        Self::deep_copy_value(v.clone())
                    };
                    super::native_api::push_handle(owned)
                }
            })
            .collect();

        let call_result = if fn_ref.raw_fn_ptr != 0 {
            // inkwell JIT path: fn pointer stored directly, no libloading needed
            unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 =
                    std::mem::transmute(fn_ref.raw_fn_ptr);
                Ok(func(handles.as_ptr(), handles.len() as i32))
            }
        } else {
            let lib = match self.native_libs.get(&fn_ref.lib_path) {
                Some(l) => l,
                None => {
                    super::native_api::abort_native_call(is_outermost);
                    return Err(format!(
                        "RuntimeError: native library not loaded: {}",
                        fn_ref.lib_path.display()
                    ));
                }
            };
            let symbol_name = format!("{}_tl\0", fn_ref.fn_name);
            unsafe {
                match lib
                    .0
                    .get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                {
                    Ok(func) => Ok(func(handles.as_ptr(), handles.len() as i32)),
                    Err(e) => Err(format!(
                        "RuntimeError: symbol '{}' not found: {e}",
                        fn_ref.fn_name
                    )),
                }
            }
        };

        match call_result {
            Err(e) => {
                super::native_api::abort_native_call(is_outermost);
                Err(e)
            }
            Ok(result_h) => {
                if let Some(err) = super::native_api::take_error() {
                    super::native_api::abort_native_call(is_outermost);
                    return Err(err);
                }
                if result_h == super::native_api::TL_EXCEPTION {
                    super::native_api::abort_native_call(is_outermost);
                    if let Some((type_name, msg)) = super::native_api::take_pending_raise() {
                        return Err(format!("{type_name}: {msg}"));
                    }
                    return Err("NativeError: CB_RAISE called but no pending raise".to_string());
                }
                // Read back write-back values BEFORE exit_native_call truncates the arena
                let updated: Vec<(String, Value)> = writebacks
                    .iter()
                    .map(|(name, h)| (name.clone(), super::native_api::clone_value_at(*h)))
                    .collect();
                let result = super::native_api::exit_native_call(result_h, is_outermost);
                // Assign updated values back to the mut variables
                for (name, val) in updated {
                    self.assign_var(&name, val)?;
                }
                Ok(result)
            }
        }
    }

    /// Core native-dispatch path: push already-evaluated args into the arena and invoke
    /// `{fn_name}_tl` either via a raw JIT pointer or via libloading.
    /// Used by both `call_native_function` (called
    /// from AST) and `call_value_with_args` (called from native callbacks).
    pub(super) fn dispatch_native_evaled(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        mut vals: Vec<Value>,
    ) -> Result<Value, String> {
        if vals.len() < fn_ref.min_params || vals.len() > fn_ref.n_params {
            let expected = if fn_ref.min_params == fn_ref.n_params {
                format!("{}", fn_ref.n_params)
            } else {
                format!("{}–{}", fn_ref.min_params, fn_ref.n_params)
            };
            return Err(format!(
                "TypeError: native function '{}' expects {} argument(s), got {}",
                fn_ref.fn_name,
                expected,
                vals.len()
            ));
        }
        while vals.len() < fn_ref.n_params {
            vals.push(Value::None);
        }

        let is_outermost = super::native_api::enter_native_call(self as *mut Interpreter);

        let handles: Vec<i64> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let is_mut = fn_ref.param_mutabilities.get(i).copied().unwrap_or(true);
                let owned = if is_mut {
                    v.clone()
                } else {
                    Self::deep_copy_value(v.clone())
                };
                super::native_api::push_handle(owned)
            })
            .collect();

        let call_result = if fn_ref.raw_fn_ptr != 0 {
            // inkwell JIT path: fn pointer stored directly, no libloading needed
            unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 =
                    std::mem::transmute(fn_ref.raw_fn_ptr);
                Ok(func(handles.as_ptr(), handles.len() as i32))
            }
        } else {
            let lib = match self.native_libs.get(&fn_ref.lib_path) {
                Some(l) => l,
                None => {
                    super::native_api::abort_native_call(is_outermost);
                    return Err(format!(
                        "RuntimeError: native library not loaded: {}",
                        fn_ref.lib_path.display()
                    ));
                }
            };
            let symbol_name = format!("{}_tl\0", fn_ref.fn_name);
            unsafe {
                match lib
                    .0
                    .get::<unsafe extern "C" fn(*const i64, i32) -> i64>(symbol_name.as_bytes())
                {
                    Ok(func) => Ok(func(handles.as_ptr(), handles.len() as i32)),
                    Err(e) => Err(format!(
                        "RuntimeError: symbol '{}' not found: {e}",
                        fn_ref.fn_name
                    )),
                }
            }
        };

        match call_result {
            Err(e) => {
                super::native_api::abort_native_call(is_outermost);
                Err(e)
            }
            Ok(result_h) => {
                if let Some(err) = super::native_api::take_error() {
                    super::native_api::abort_native_call(is_outermost);
                    return Err(err);
                }
                if result_h == super::native_api::TL_EXCEPTION {
                    super::native_api::abort_native_call(is_outermost);
                    if let Some((type_name, msg)) = super::native_api::take_pending_raise() {
                        return Err(format!("{type_name}: {msg}"));
                    }
                    return Err("NativeError: CB_RAISE called but no pending raise".to_string());
                }
                Ok(super::native_api::exit_native_call(result_h, is_outermost))
            }
        }
    }

    // --- ネイティブコールバック用ヘルパー ---

    /// Resolve an attribute on any `Value`.
    /// Used by both `eval_attr` (from AST) and native callbacks (`ar_get_attr`).
    pub(super) fn get_attr_val(&mut self, obj: Value, attr: &str) -> Result<Value, String> {
        match &obj {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let cls = inst.class.clone();
                if let Some((v, _)) = inst.fields.get(attr) {
                    let v = v.clone();
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(v);
                }
                let suffix = format!("::{attr}");
                if let Some((full_key, (v, _))) = inst
                    .fields
                    .iter()
                    .find(|(k, _)| k.ends_with(suffix.as_str()))
                {
                    let v = v.clone();
                    let full_key = full_key.clone();
                    drop(inst);
                    self.check_member_access(&cls, &full_key, attr)?;
                    return Ok(v);
                }
                if let Some(v) = Self::lookup_class_var(&cls, attr) {
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(v);
                }
                if let Some(cell) = cls.static_vars.get(attr).cloned() {
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(cell.borrow().clone());
                }
                if cls.methods.contains_key(attr) {
                    if cls.static_method_names.contains(attr) {
                        drop(inst);
                        return Err(format!(
                            "AttributeError: static method '{}' is not accessible on an instance of '{}'; use '{}.{}'",
                            attr, cls.name, cls.name, attr
                        ));
                    }
                    if cls.class_method_names.contains(attr) {
                        drop(inst);
                        return Err(format!(
                            "AttributeError: class method '{}' is not accessible on an instance of '{}'; use '{}.{}'",
                            attr, cls.name, cls.name, attr
                        ));
                    }
                    let overloads = cls.methods.get(attr).unwrap();
                    let result = if overloads.len() == 1 {
                        Value::Function(overloads[0].clone())
                    } else {
                        Value::OverloadedFn(overloads.clone())
                    };
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(result);
                }
                Err(format!(
                    "AttributeError: '{}' object has no attribute '{attr}'",
                    cls.name
                ))
            }
            Value::Class(cls) => {
                if attr == "name" {
                    return Ok(Value::Str(cls.name.clone()));
                }
                if let Some(v) = Self::lookup_class_var(cls, attr) {
                    return Ok(v);
                }
                if let Some(cell) = cls.static_vars.get(attr) {
                    return Ok(cell.borrow().clone());
                }
                if let Some(overloads) = cls.methods.get(attr) {
                    return Ok(if overloads.len() == 1 {
                        Value::Function(overloads[0].clone())
                    } else {
                        Value::OverloadedFn(overloads.clone())
                    });
                }
                Err(format!(
                    "AttributeError: class '{}' has no attribute '{attr}'",
                    cls.name
                ))
            }
            Value::Namespace(ns) => ns.members.get(attr).cloned().ok_or_else(|| {
                format!(
                    "AttributeError: module '{}' has no attribute '{attr}'",
                    ns.name
                )
            }),
            Value::PyObject(handle) => super::py_interop::py_getattr(handle, attr),
            Value::Slice(s) => match attr {
                "begin" => Ok(s.begin.clone().unwrap_or(Value::None)),
                "end" => Ok(s.end.clone().unwrap_or(Value::None)),
                "step" => Ok(s.step.clone().unwrap_or(Value::None)),
                _ => Err(format!("AttributeError: 'slice' has no attribute '{attr}'")),
            },
            Value::Signal(sig_rc) => {
                // Signal[T] には read-only プロパティのみ。メソッドは eval_method_call が処理する。
                let sig = sig_rc.borrow();
                match attr {
                    "handler_count" => Ok(Value::Int(sig.handlers.len() as i64)),
                    _ => Err(format!(
                        "AttributeError: 'Signal' object has no attribute '{attr}'"
                    )),
                }
            }
            Value::EventLoop(_) => {
                // EventLoop のメソッドは eval_method_call が処理する。属性アクセスのみここ。
                Err(format!(
                    "AttributeError: 'EventLoop' object has no attribute '{attr}' (use EventLoop.run() / EventLoop.post())"
                ))
            }
            Value::AsyncManager(mgr_rc) => {
                let mgr = mgr_rc.borrow();
                match attr {
                    "num_thread" => Ok(Value::UInt(mgr.num_thread as u64)),
                    "raise_immediately" => Ok(Value::Bool(mgr.raise_immediately)),
                    "thread_status" => {
                        let running: Vec<Value> = mgr
                            .progress
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| **s == super::async_mgr::AsyncStatus::Running)
                            .map(|(i, _)| Value::Int(i as i64))
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(running))))
                    }
                    "progress_status" => {
                        let statuses: Vec<Value> = mgr
                            .progress
                            .iter()
                            .map(|s| Value::AsyncStatusVal(s.clone()))
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(statuses))))
                    }
                    "results" => Ok(Value::List(Rc::new(RefCell::new(mgr.results.clone())))),
                    "error_list" => {
                        let errs: Vec<Value> = mgr
                            .error_list
                            .iter()
                            .map(|e| match e {
                                Some(s) => Value::Str(s.clone()),
                                None => Value::None,
                            })
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(errs))))
                    }
                    _ => Err(format!(
                        "AttributeError: 'AsyncManager' has no attribute '{attr}'"
                    )),
                }
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no attribute '{attr}'",
                self.type_name(&obj)
            )),
        }
    }

    /// 任意の呼び出し可能な `Value` を評価済み引数リストで呼び出す。
    /// ネイティブコールバック `ar_call_fn` から呼ばれる。
    pub(super) fn call_value_with_args(
        &mut self,
        callee: Value,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        let evaled: Vec<(Option<String>, Value)> = args.into_iter().map(|v| (None, v)).collect();
        match callee {
            Value::Function(fn_val) => self.exec_fn_evaled(fn_val, &evaled, None, "<fn>", None),
            Value::OverloadedFn(candidates) => {
                self.dispatch_overload_evaled(candidates, evaled, None, "<overloaded>", None)
            }
            Value::Class(cls) => {
                // Class constructor called from native code (e.g. `Point(x, y)` via cb_call).
                self.instantiate_evaled(cls, evaled)
            }
            Value::NativeFunction(fn_ref) => {
                let vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
                self.dispatch_native_evaled(&fn_ref, vals)
            }
            Value::Type(type_name) => {
                let vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
                self.call_type_by_name_evaled(&type_name, vals)
            }
            Value::Instance(_) => self.eval_method_call_evaled(callee, "__call__", evaled),
            Value::PyObject(ref handle) => super::py_interop::call_py_object(handle, &evaled),
            other => Err(format!(
                "TypeError: '{}' object is not callable",
                self.type_name(&other)
            )),
        }
    }

    /// AsyncManager(num_thread=N [, raise_immediately=bool]) コンストラクタ
    fn make_async_manager(&mut self, args: &[crate::ast::CallArg]) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;

        let mut num_thread: usize = 1;
        let mut raise_immediately: bool = false;

        match evaled.as_slice() {
            [] => {}
            _ => {
                for (kw, val) in &evaled {
                    match kw.as_deref() {
                        Some("num_thread") | None => {
                            match val {
                                Value::Int(n) if *n > 0 => num_thread = *n as usize,
                                Value::UInt(n) => num_thread = *n as usize,
                                other => return Err(format!(
                                    "TypeError: AsyncManager num_thread must be a positive int, got '{}'",
                                    self.type_name(other)
                                )),
                            }
                        }
                        Some("raise_immediately") => {
                            match val {
                                Value::Bool(b) => raise_immediately = *b,
                                other => return Err(format!(
                                    "TypeError: AsyncManager raise_immediately must be bool, got '{}'",
                                    self.type_name(other)
                                )),
                            }
                        }
                        Some(k) => return Err(format!(
                            "TypeError: AsyncManager() got unexpected keyword argument '{k}'"
                        )),
                    }
                }
            }
        }

        let mgr = super::async_mgr::AsyncManagerData::new(num_thread, raise_immediately);
        Ok(Value::AsyncManager(Rc::new(RefCell::new(mgr))))
    }

    /// インスタンスの属性に値をセットする。
    /// ネイティブコールバック `ar_set_attr` から呼ばれる。
    pub(super) fn set_attr_val(
        &mut self,
        obj: Value,
        attr: &str,
        val: Value,
    ) -> Result<(), String> {
        match obj {
            Value::Instance(inst_rc) => {
                let inst_class = inst_rc.borrow().class.clone();
                if Self::lookup_class_var(&inst_class, attr).is_some() {
                    return Err(format!(
                        "TypeError: cannot assign to class variable '{attr}' (declared const)"
                    ));
                }
                self.check_member_access(&inst_class, attr, attr)?;
                let mut inst = inst_rc.borrow_mut();
                if let Some((_, false)) = inst.fields.get(attr) {
                    return Err(format!(
                        "TypeError: cannot assign to immutable field '{attr}'"
                    ));
                }
                if inst.immutable {
                    return Err(format!(
                        "TypeError: cannot assign field '{attr}' on immutable instance"
                    ));
                }
                let is_mutable = inst
                    .class
                    .field_mutability
                    .get(attr)
                    .copied()
                    .unwrap_or(true);
                inst.fields.insert(attr.to_string(), (val, is_mutable));
                Ok(())
            }
            _ => Err("AttributeError: cannot set attribute on non-instance".to_string()),
        }
    }

    // --- 属性代入ヘルパー ---

    /// 属性・添字に値を代入する。`AttrAssign` 文と `AttrCompoundAssign` 文から呼ばれる。
    ///
    // ---------------------------------------------------------------------------
    // ブロック式 / 制御フロー式 の共通ヘルパー
    // ---------------------------------------------------------------------------

    /// block: 式 / Expr::Block の実体。BLOCK_YIELDS コンテキストを退避・復元しながら実行する。
    pub(super) fn eval_block_expr(&mut self, stmts: &[crate::ast::Stmt]) -> Result<Value, String> {
        let saved = BLOCK_YIELDS.with(|y| y.borrow_mut().take());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));

        self.push_scope();
        let mut block_return_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'block_expr: for stmt in stmts {
            match self.exec(stmt) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::BlockReturn(v)) => {
                    block_return_val = Some(v);
                    break 'block_expr;
                }
                Ok(ExecResult::BlockYield(_)) => {} // スレッドローカル経由で収集済み
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'block_expr;
                }
                Ok(ExecResult::Return(_)) => {
                    early_err = Some(
                        "SyntaxError: 'return' inside block expression — use 'block_return'"
                            .to_string(),
                    );
                    break 'block_expr;
                }
                Ok(ExecResult::Break) => {
                    // break propagates through block: expressions to reach the enclosing loop
                    early_err = Some(BREAK_SENTINEL.to_string());
                    break 'block_expr;
                }
                Ok(ExecResult::Continue) => {
                    early_err = Some("SyntaxError: 'continue' inside block expression is not supported outside a loop".to_string());
                    break 'block_expr;
                }
                Err(e) => {
                    early_err = Some(e);
                    break 'block_expr;
                }
            }
        }
        self.pop_scope();

        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err {
            return Err(e);
        }
        match block_return_val {
            Some(v) => Ok(v),
            None => {
                if yields.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(Value::List(Rc::new(RefCell::new(yields))))
                }
            }
        }
    }

    /// IfExpr のブランチ評価ロジック。BLOCK_RETURN_EXPECTED_TYPE の push/pop は呼び出し元が行う。
    fn eval_if_expr_body(
        &mut self,
        branches: &[(crate::ast::Expr, Vec<crate::ast::Stmt>)],
        else_body: &Option<Vec<crate::ast::Stmt>>,
    ) -> Result<Value, String> {
        for (cond, body) in branches {
            let val = self.eval(cond)?;
            if self.eval_truthy(&val)? {
                return self.eval_capture_block_return(body);
            }
        }
        if let Some(body) = else_body {
            return self.eval_capture_block_return(body);
        }
        Ok(Value::None)
    }

    /// if / match 式のボディを実行し、BlockReturn シグナルを値として捕捉して返す。
    /// BLOCK_YIELDS は設定しない（透過的 — 外側の for/while/block 式に yield が届く）。
    pub(super) fn eval_capture_block_return(
        &mut self,
        stmts: &[crate::ast::Stmt],
    ) -> Result<Value, String> {
        self.push_scope();
        let mut result_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'body: for stmt in stmts {
            match self.exec(stmt) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::BlockReturn(v)) => {
                    result_val = Some(v);
                    break 'body;
                }
                Ok(ExecResult::Break) => {
                    // break propagates through if/match expressions to reach the enclosing loop
                    early_err = Some(BREAK_SENTINEL.to_string());
                    break 'body;
                }
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'body;
                }
                Ok(ExecResult::Return(_)) => {
                    early_err = Some(
                        "SyntaxError: 'return' inside block expression — use 'block_return'"
                            .to_string(),
                    );
                    break 'body;
                }
                Ok(other) => {
                    let _ = other;
                }
                Err(e) => {
                    early_err = Some(e);
                    break 'body;
                }
            }
        }
        self.pop_scope();
        if let Some(e) = early_err {
            return Err(e);
        }
        Ok(result_val.unwrap_or(Value::None))
    }

    /// for 式の実体。BLOCK_YIELDS コンテキストと LOOP_DEPTH を管理し、loop_yield でリスト蓄積、block_return で単値返却。
    pub(super) fn eval_for_expr(
        &mut self,
        target: &str,
        iter_expr: &crate::ast::Expr,
        body: &[crate::ast::Stmt],
    ) -> Result<Value, String> {
        let iter_val = self.eval(iter_expr)?;
        let generator = match iter_val {
            Value::List(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::Set(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: chars,
                    index: 0,
                })))
            }
            Value::Generator(_) => iter_val,
            Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[])?,
            Value::PyObject(ref handle) => {
                let items = super::py_interop::py_collect_iter(handle)?;
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: items,
                    index: 0,
                })))
            }
            _ => return Err("TypeError: object is not iterable".to_string()),
        };

        let saved = BLOCK_YIELDS.with(|y| y.borrow_mut().take());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);

        let mut block_return_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'for_loop: loop {
            match self.eval_method_call(generator.clone(), "next", &[]) {
                Ok(item) => {
                    self.push_scope();
                    self.declare_var(target.to_string(), Var::new(item, true));
                    let result = self.exec_block(body);
                    self.pop_scope();
                    match result {
                        Ok(ExecResult::Normal) => {}
                        Ok(ExecResult::Continue) => continue,
                        // break: exit loop and return accumulated loop_yields (or None)
                        Ok(ExecResult::Break) => break 'for_loop,
                        // block_return None: exit loop with explicit None (ignores yields)
                        Ok(ExecResult::BlockReturn(Value::None)) => {
                            block_return_val = Some(Value::None);
                            break 'for_loop;
                        }
                        Ok(ExecResult::BlockReturn(v)) => {
                            block_return_val = Some(v);
                            break 'for_loop;
                        }
                        Ok(ExecResult::Raise(raised)) => {
                            self.current_exception = Some(raised);
                            early_err = Some(RAISE_SENTINEL.to_string());
                            break 'for_loop;
                        }
                        Ok(ExecResult::Return(v)) => {
                            block_return_val = Some(v);
                            break 'for_loop;
                        } // shouldn't happen
                        Ok(ExecResult::BlockYield(_)) => {}
                        // break from inside an eval context (e.g. if expression body)
                        Err(ref e) if e.as_str() == BREAK_SENTINEL => break 'for_loop,
                        Err(e) => {
                            early_err = Some(e);
                            break 'for_loop;
                        }
                    }
                }
                Err(ref e) if e.starts_with("EndOfIteration") => break,
                Err(e) => {
                    early_err = Some(e);
                    break;
                }
            }
        }

        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err {
            return Err(e);
        }
        match block_return_val {
            Some(v) => Ok(v),
            None => {
                if yields.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(Value::List(Rc::new(RefCell::new(yields))))
                }
            }
        }
    }

    /// while 式の実体。for 式と同様に BLOCK_YIELDS と LOOP_DEPTH を管理する。
    pub(super) fn eval_while_expr(
        &mut self,
        cond_expr: &crate::ast::Expr,
        body: &[crate::ast::Stmt],
    ) -> Result<Value, String> {
        let saved = BLOCK_YIELDS.with(|y| y.borrow_mut().take());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);

        let mut block_return_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'while_loop: loop {
            let cond_val = match self.eval(cond_expr) {
                Ok(v) => v,
                Err(e) => {
                    early_err = Some(e);
                    break;
                }
            };
            match self.eval_truthy(&cond_val) {
                Ok(false) => break,
                Ok(true) => {}
                Err(e) => { early_err = Some(e); break 'while_loop; }
            }

            match self.exec_scoped_block(body) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::Continue) => continue,
                // break: exit loop and return accumulated loop_yields (or None)
                Ok(ExecResult::Break) => break 'while_loop,
                // block_return None: exit loop with explicit None (ignores yields)
                Ok(ExecResult::BlockReturn(Value::None)) => {
                    block_return_val = Some(Value::None);
                    break 'while_loop;
                }
                Ok(ExecResult::BlockReturn(v)) => {
                    block_return_val = Some(v);
                    break 'while_loop;
                }
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'while_loop;
                }
                Ok(other) => {
                    let _ = other;
                }
                // break from inside an eval context (e.g. if expression body)
                Err(ref e) if e.as_str() == BREAK_SENTINEL => break 'while_loop,
                Err(e) => {
                    early_err = Some(e);
                    break;
                }
            }
        }

        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err {
            return Err(e);
        }
        match block_return_val {
            Some(v) => Ok(v),
            None => {
                if yields.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(Value::List(Rc::new(RefCell::new(yields))))
                }
            }
        }
    }

    /// 対応する代入ターゲット:
    /// - `Expr::Attr { object, attr }`: インスタンスフィールドへの代入（可変性・const チェック付き）
    /// - `Expr::TraitAccess { object, trait_name, attr }`: トレイトフィールドへの代入
    /// - `Expr::Subscript { object, index }`: 辞書への添字代入（型制約チェック付き）
    ///
    /// - `target`: 代入先の式（`Attr` / `TraitAccess` / `Subscript`）
    /// - `rhs`: 代入する値（評価済み）
    ///
    /// 戻り値: `Ok(())` — 成功。`Err(message)` — 型エラー・不変フィールドへの代入エラー等
    pub(super) fn attr_assign(&mut self, target: &Expr, rhs: Value) -> Result<(), String> {
        if let Expr::Attr { object, attr, .. } = target {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    let inst_class = inst_rc.borrow().class.clone();
                    if Self::lookup_class_var(&inst_class, attr).is_some() {
                        return Err(format!(
                            "TypeError: cannot assign to class variable '{attr}' (declared const)"
                        ));
                    }
                    // static mut 変数への代入: 共有セルを更新する
                    if let Some(cell) = inst_class.static_vars.get(attr.as_str()).cloned() {
                        self.check_member_access(&inst_class, attr, attr)?;
                        *cell.borrow_mut() = rhs;
                        return Ok(());
                    }
                    // アクセス制御チェック
                    self.check_member_access(&inst_class, attr, attr)?;
                    let mut inst = inst_rc.borrow_mut();
                    if let Some((_, mutable)) = inst.fields.get(attr.as_str()) {
                        if !mutable {
                            return Err(format!(
                                "TypeError: cannot assign to immutable field '{attr}'"
                            ));
                        }
                        inst.fields.insert(attr.clone(), (rhs, true));
                    } else {
                        if inst.immutable {
                            return Err(format!(
                                "TypeError: cannot assign field '{attr}' on immutable instance"
                            ));
                        }
                        let is_mutable = inst
                            .class
                            .field_mutability
                            .get(attr.as_str())
                            .copied()
                            .unwrap_or(true);
                        inst.fields.insert(attr.clone(), (rhs, is_mutable));
                    }
                    Ok(())
                }
                Value::Class(cls) => {
                    // クラスオブジェクトへの代入: static mut 変数のみ許可
                    if let Some(cell) = cls.static_vars.get(attr.as_str()).cloned() {
                        *cell.borrow_mut() = rhs;
                        return Ok(());
                    }
                    if Self::lookup_class_var(&cls, attr).is_some() {
                        return Err(format!(
                            "TypeError: cannot assign to class variable '{attr}' (declared const)"
                        ));
                    }
                    Err(format!(
                        "AttributeError: class '{}' has no static field '{attr}'",
                        cls.name
                    ))
                }
                _ => Err("AttributeError: cannot set attribute on non-instance".to_string()),
            }
        } else if let Expr::TraitAccess {
            object,
            trait_name,
            attr,
        } = target
        {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    // Trait fields are stored with a namespaced key "TraitName::field"
                    let key = format!("{}::{}", trait_name, attr);
                    let inst_class = inst_rc.borrow().class.clone();
                    // アクセス制御チェック（トレイトフィールドのキーで検索）
                    self.check_member_access(&inst_class, &key, attr)?;
                    let mut inst = inst_rc.borrow_mut();
                    if let Some((_, false)) = inst.fields.get(&key) {
                        return Err(format!(
                            "TypeError: cannot assign to immutable trait field '{attr}'"
                        ));
                    }
                    if inst.immutable {
                        return Err(format!(
                            "TypeError: cannot assign field '{attr}' on immutable instance"
                        ));
                    }
                    inst.fields.insert(key, (rhs, true));
                    Ok(())
                }
                _ => Err("AttributeError: cannot set trait field on non-instance".to_string()),
            }
        } else if let Expr::Subscript { object, index } = target {
            let obj_val = self.eval(object)?;
            let key = self.eval(index)?;
            self.eval_setitem(obj_val, key, rhs)
        } else {
            Err("SyntaxError: invalid assignment target".to_string())
        }
    }

    /// 値が宣言された型名と互換性があるかを確認する。
    ///
    /// 特別ルール:
    /// - `"Any"` はすべての型を受け入れる
    /// - `"float"` には `int` 値も受け入れる（アップキャスト）
    /// - それ以外のユーザー定義型はクラス名で比較する
    ///
    /// - `val`: チェック対象の値
    /// - `type_name`: 宣言された型名
    ///
    /// 戻り値: `true` — 互換あり、`false` — 型不一致
    pub(super) fn value_matches_type(val: &Value, type_name: &str) -> bool {
        match type_name {
            "Any" => true,
            "int" => matches!(val, Value::Int(_)),
            "float" => matches!(val, Value::Float(_) | Value::Int(_)),
            "str" => matches!(val, Value::Str(_)),
            "bool" => matches!(val, Value::Bool(_)),
            "None" => matches!(val, Value::None),
            "list" => matches!(val, Value::List(_)),
            "fixed_list" => matches!(val, Value::FrozenList { .. }),
            "list_like" => matches!(val, Value::List(_) | Value::FrozenList { .. }),
            "dict" => matches!(val, Value::Dict(_)),
            "set" => matches!(val, Value::Set(_)),
            "tuple" => matches!(val, Value::Tuple(_)),
            _ if type_name.starts_with("list[") => matches!(val, Value::List(_)),
            _ if type_name.starts_with("fixed_list[") => matches!(val, Value::FrozenList { .. }),
            _ if type_name.starts_with("list_like[") => matches!(val, Value::List(_) | Value::FrozenList { .. }),
            _ if type_name.starts_with("dict[") => matches!(val, Value::Dict(_)),
            _ if type_name.starts_with("set[") => matches!(val, Value::Set(_)),
            _ if type_name.starts_with("tuple[") => matches!(val, Value::Tuple(_)),
            _ => {
                if let Value::Instance(inst) = val {
                    inst.borrow().class.name == type_name
                } else {
                    false
                }
            }
        }
    }

    /// `obj[key]` の評価。リスト・文字列・タプル・辞書・PyObject・インスタンスに対応する。
    /// `key` が `Value::Slice` の場合はスライス処理を行い、新たなリスト/文字列/タプルを返す。
    pub(super) fn eval_subscript(&mut self, obj: Value, key: Value) -> Result<Value, String> {
        if let Value::Slice(s) = &key {
            return self.eval_subscript_slice(obj, Rc::clone(s));
        }
        match obj {
            Value::List(items) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: list indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let borrowed = items.borrow();
                let len = borrowed.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: list index {} out of range", idx));
                }
                Ok(borrowed[actual as usize].clone())
            }
            Value::FrozenList { ref state, ref layout } => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: list indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let st = state.borrow();
                let n = st.len as i64;
                let actual = if idx < 0 { n + idx } else { idx };
                if actual < 0 || actual >= n {
                    return Err(format!("IndexError: list index {} out of range", idx));
                }
                Ok(layout.reconstruct_item(&st.data, actual as usize))
            }
            Value::Str(s) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: string indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: string index {} out of range", idx));
                }
                Ok(Value::Str(chars[actual as usize].to_string()))
            }
            Value::Tuple(td) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: tuple indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let vals = td.all_values();
                let len = vals.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: tuple index {} out of range", idx));
                }
                Ok(vals[actual as usize].clone())
            }
            Value::Dict(d) => d
                .borrow()
                .get(&key)
                .ok_or_else(|| format!("KeyError: {}", self.display(&key))),
            Value::Instance(_) => {
                self.eval_method_call_evaled(obj, "__getitem__", vec![(None, key)])
            }
            Value::PyObject(ref handle) => super::py_interop::py_getitem(handle, &key),
            _ => Err(format!(
                "TypeError: '{}' object is not subscriptable",
                self.type_name(&obj)
            )),
        }
    }

    /// スライス添字 `obj[begin:end:step]` を評価する。
    /// リスト → 新しいリスト、文字列 → 新しい文字列、タプル → 新しいタプルを返す。
    fn eval_subscript_slice(&mut self, obj: Value, s: Rc<SliceValue>) -> Result<Value, String> {
        let step = match &s.step {
            None => 1i64,
            Some(Value::Int(n)) => *n,
            _ => return Err("TypeError: slice step must be int".to_string()),
        };
        if step == 0 {
            return Err("ValueError: slice step cannot be zero".to_string());
        }
        let begin = index_val_to_i64(&s.begin);
        let end = index_val_to_i64(&s.end);

        match obj {
            Value::List(items) => {
                let borrowed = items.borrow();
                let len = borrowed.len() as i64;
                let indices = compute_slice_indices(len, begin, end, step);
                Ok(Value::List(Rc::new(RefCell::new(
                    indices.into_iter().map(|i| borrowed[i].clone()).collect(),
                ))))
            }
            Value::Str(s_val) => {
                let chars: Vec<char> = s_val.chars().collect();
                let len = chars.len() as i64;
                let indices = compute_slice_indices(len, begin, end, step);
                Ok(Value::Str(indices.into_iter().map(|i| chars[i]).collect()))
            }
            Value::Tuple(td) => {
                let vals = td.all_values();
                let types: Vec<String> =
                    vals.iter().map(|v| self.type_name(v).to_string()).collect();
                let len = vals.len() as i64;
                let indices = compute_slice_indices(len, begin, end, step);
                let new_vals: Vec<Value> = indices.iter().map(|&i| vals[i].clone()).collect();
                let new_types: Vec<String> = indices.iter().map(|&i| types[i].clone()).collect();
                Ok(Value::Tuple(Rc::new(TupleData::new(new_vals, new_types))))
            }
            // カスタムクラス: __getitem__ にスライスオブジェクトを渡して委譲する
            Value::Instance(_) => {
                self.eval_method_call_evaled(obj, "__getitem__", vec![(None, Value::Slice(s))])
            }
            _ => Err(format!(
                "TypeError: '{}' object does not support slicing",
                self.type_name(&obj)
            )),
        }
    }

    /// `obj[slice] = rhs` を実行する。
    /// - `Value::List`: Python 互換のスライス代入（step=1 は長さ変更可、step≠1 は同数必須）
    /// - `Value::Instance`: `__setitem__(slice, rhs)` に委譲する
    fn eval_setitem_slice(
        &mut self,
        obj: Value,
        s: Rc<SliceValue>,
        rhs: Value,
    ) -> Result<(), String> {
        let step = match &s.step {
            None => 1i64,
            Some(Value::Int(n)) => *n,
            _ => return Err("TypeError: slice step must be int".to_string()),
        };
        if step == 0 {
            return Err("ValueError: slice step cannot be zero".to_string());
        }
        let begin = index_val_to_i64(&s.begin);
        let end = index_val_to_i64(&s.end);

        match obj {
            Value::List(items) => {
                let new_vals = self.collect_iterable(rhs)?;
                let mut borrowed = items.borrow_mut();
                let len = borrowed.len() as i64;

                if step == 1 {
                    // step=1: Python 互換。置換先の長さと代入元の長さが違っても構わない。
                    let start = normalize_slice_bound_start(begin, len);
                    let stop = normalize_slice_bound_stop(end, len);
                    // start > stop のときは空スライスへの挿入（Python の動作と一致）
                    let stop = stop.max(start);
                    borrowed.splice(start..stop, new_vals);
                } else {
                    // step≠1: 拡張スライス。代入元の要素数がスライスの要素数と一致しなければならない。
                    let indices = compute_slice_indices(len, begin, end, step);
                    if new_vals.len() != indices.len() {
                        return Err(format!(
                            "ValueError: attempt to assign sequence of size {} to extended slice of size {}",
                            new_vals.len(), indices.len()
                        ));
                    }
                    for (new_val, &idx) in new_vals.into_iter().zip(indices.iter()) {
                        borrowed[idx] = new_val;
                    }
                }
                Ok(())
            }
            // カスタムクラス: __setitem__ にスライスオブジェクトと値を渡して委譲する
            Value::Instance(_) => {
                self.eval_method_call_evaled(
                    obj,
                    "__setitem__",
                    vec![(None, Value::Slice(s)), (None, rhs)],
                )?;
                Ok(())
            }
            _ => Err(format!(
                "TypeError: '{}' object does not support slice assignment",
                self.type_name(&obj)
            )),
        }
    }

    /// 任意の反復可能値を `Vec<Value>` に収集する（スライス代入、enumerate、zip で使用）。
    fn collect_iterable(&self, val: Value) -> Result<Vec<Value>, String> {
        match val {
            Value::List(lst) => Ok(lst.borrow().clone()),
            Value::FrozenList { ref state, ref layout } => {
                let st = state.borrow();
                Ok((0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect())
            }
            Value::Tuple(td) => Ok(td.all_values().to_vec()),
            Value::Str(s) => Ok(s.chars().map(|c| Value::Str(c.to_string())).collect()),
            Value::Set(items) => Ok(items.borrow().clone()),
            Value::Generator(gen) => {
                let g = gen.borrow();
                Ok(g.values[g.index..].to_vec())
            }
            other => Err(format!(
                "TypeError: '{}' object is not iterable",
                self.type_name(&other)
            )),
        }
    }

    /// `obj[key] = rhs` の実行。リスト・辞書・PyObject・インスタンスに対応する。
    /// `key` が `Value::Slice` の場合はスライス代入 `eval_setitem_slice` に委譲する。
    pub(super) fn eval_setitem(
        &mut self,
        obj: Value,
        key: Value,
        rhs: Value,
    ) -> Result<(), String> {
        if let Value::Slice(s) = key {
            return self.eval_setitem_slice(obj, s, rhs);
        }
        match obj {
            Value::List(items) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: list indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let mut borrowed = items.borrow_mut();
                let len = borrowed.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!(
                        "IndexError: list assignment index {} out of range",
                        idx
                    ));
                }
                borrowed[actual as usize] = rhs;
                Ok(())
            }
            Value::Dict(d) => {
                let (key_type, item_type) = {
                    let b = d.borrow();
                    (b.key_type.clone(), b.item_type.clone())
                };
                if key_type != "Any" && !Self::value_matches_type(&key, &key_type) {
                    return Err(format!(
                        "TypeError: dict key type mismatch: expected '{}', got '{}'",
                        key_type,
                        self.type_name(&key)
                    ));
                }
                if item_type != "Any" && !Self::value_matches_type(&rhs, &item_type) {
                    return Err(format!(
                        "TypeError: dict item type mismatch: expected '{}', got '{}'",
                        item_type,
                        self.type_name(&rhs)
                    ));
                }
                d.borrow_mut().set(key, rhs);
                Ok(())
            }
            Value::Instance(_) => {
                self.eval_method_call_evaled(obj, "__setitem__", vec![(None, key), (None, rhs)])?;
                Ok(())
            }
            Value::PyObject(ref handle) => super::py_interop::py_setitem(handle, &key, &rhs),
            _ => Err(format!(
                "TypeError: '{}' object does not support item assignment",
                self.type_name(&obj)
            )),
        }
    }
}
