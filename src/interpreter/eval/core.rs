// eval/core.rs — 式評価のコア: eval 本体のディスパッチと、トレイトアクセス・属性・スライス・二項演算・match 式の評価。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::{BinOp, Expr, MatchArm, MatchPattern},
    crate::interpreter::{
        DictData,
        Interpreter, SliceValue, TupleData, Value,
        BLOCK_RETURN_EXPECTED_TYPE, RAISE_SENTINEL,
    },
};
use super::*;

impl Interpreter {
    /// VM: リテラルから List を構築する（`Expr::List` と同一意味論）。
    pub(crate) fn vm_build_list(&self, vals: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(vals)))
    }

    /// VM: リテラルから Tuple を構築する（`Expr::Tuple` と同一・要素型名を収集）。
    pub(crate) fn vm_build_tuple(&self, vals: Vec<Value>) -> Value {
        let types: Vec<String> = vals.iter().map(|v| self.type_name(v).to_string()).collect();
        Value::Tuple(Rc::new(TupleData::new(vals, types)))
    }

    /// VM: リテラルから Set を構築する（`Expr::Set` と同一・`set_insert` で重複排除）。
    pub(crate) fn vm_build_set(&self, vals: Vec<Value>) -> Value {
        let mut out: Vec<Value> = Vec::new();
        for v in vals {
            set_insert(&mut out, v, self);
        }
        Value::Set(Rc::new(RefCell::new(out)))
    }

    /// VM: リテラルから Dict を構築する（`Expr::Dict` と同一）。`flat` は `[k0,v0,k1,v1,..]`。
    pub(crate) fn vm_build_dict(&self, flat: Vec<Value>) -> Value {
        let mut d = DictData::new("Any".to_string(), "Any".to_string());
        let mut it = flat.into_iter();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            d.set(k, v);
        }
        Value::Dict(Rc::new(RefCell::new(d)))
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
            Expr::Undefined => Ok(Value::Undefined),
            Expr::Ident(name) => self
                .get_val(name)
                .ok_or_else(|| format!("NameError: '{name}' is not defined")),
            Expr::LocalRef { name, slot } => self.eval_local_ref(name, *slot),
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
            Expr::Attr { object, attr, cache, .. } => self.eval_attr(object, attr, cache),
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
            Expr::MustBe { expr, guard_type, span, .. } => {
                let val = self.eval(expr)?;
                let outer = mustbe_outer_type(guard_type);
                if self.value_is_type(&val, &outer) {
                    Ok(val)
                } else {
                    let actual = self.type_name_of(&val);
                    let msg = format!(
                        "TypeError: mustbe assertion failed at {}: expected `{}`, got `{}`",
                        span, guard_type, actual
                    );
                    if let Some(raised) = self.make_internal_raised_error(&msg) {
                        self.current_exception = Some(raised);
                        Err(RAISE_SENTINEL.to_string())
                    } else {
                        Err(msg)
                    }
                }
            }
            Expr::Call { func, args, span, cache } => self.eval_call(func, args, span, cache),
            Expr::Cast { object, type_name, .. } => self.eval_cast(object, type_name),
        }
    }

    /// 解決済みローカル参照（`Expr::LocalRef`）の高速読み取り（Phase R / R1）。
    ///
    /// リゾルバは、トップレベル関数の base スコープに確実に解決できる読み取りだけを書き換える。
    /// base スコープは実行時 `scopes[frame_floor]`（関数フレームの底）に来るので、
    /// `scopes[frame_floor].slot(slot)` を index 1回で読める（スコープ遡り・文字列ハッシュなし）。
    /// デバッグビルドでは slot と名前の一致を検証し、リゾルバのずれを即座に露見させる。
    /// 想定外（境界外など）の場合のみ名前引きへフォールバックして正しさを保つ。
    #[inline]
    pub(crate) fn eval_local_ref(&self, name: &str, slot: u32) -> Result<Value, String> {
        let s = slot as usize;
        if let Some(scope) = self.scopes.get(self.frame_floor) {
            if let Some(var) = scope.slot(s) {
                debug_assert_eq!(
                    scope.slot_of(name),
                    Some(s),
                    "LocalRef slot mismatch for '{name}': resolver said slot {s}, \
                     runtime index says {:?}",
                    scope.slot_of(name)
                );
                return Ok(var.get_value());
            }
        }
        self.get_val(name)
            .ok_or_else(|| format!("NameError: '{name}' is not defined"))
    }

    // --- eval() から抽出したメソッド群 ---

    /// トレイトアクセス式 `obj:TraitName::attr` を評価する。
    /// インスタンスのフィールドマップから名前空間付きキー `TraitName::attr` を検索して返す。
    pub(crate) fn eval_trait_access(
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
                if let Some(&idx) = inst.class.field_index.get(&key) {
                    if let Some(v) = inst.field_value(idx) {
                        return Ok(v);
                    }
                }
                Err(format!(
                    "AttributeError: trait field '{trait_name}::{attr}' not found on '{}'",
                    inst.class.name
                ))
            }
            _ => Err("AttributeError: cannot access trait field on non-instance".to_string()),
        }
    }

    /// 属性アクセス式 `obj.attr` を評価する。
    ///
    /// R3 インラインキャッシュ: インスタンスの own/unqualified フィールドで `class_id` が
    /// キャッシュと一致すれば、`field_index` の辞書引き・アクセスキー走査・`format!` 確保を
    /// 飛ばして slot を直接読む。ミス時は `get_attr_val` で解決してキャッシュを更新する。
    pub(crate) fn eval_attr(
        &mut self,
        object: &Expr,
        attr: &str,
        cache: &crate::ast::AttrCache,
    ) -> Result<Value, String> {
        let obj_val = self.eval(object)?;
        if let Value::Instance(inst_rc) = &obj_val {
            let class_id = inst_rc.borrow().class.class_id;
            if let Some((idx, access)) = cache.get(class_id) {
                let inst = inst_rc.borrow();
                debug_assert_eq!(
                    inst.class.field_index.get(attr).copied(),
                    Some(idx),
                    "AttrCache slot mismatch for '{attr}' on class_id {class_id}"
                );
                if let Some(v) = inst.field_value(idx) {
                    if access == crate::ast::AttrCache::PUBLIC {
                        return Ok(v);
                    }
                    let cls = inst.class.clone();
                    drop(inst);
                    self.check_access_level(&cls, access, attr)?;
                    return Ok(v);
                }
                // 未初期化 slot 等の想定外ケースは通常経路へ委譲する。
            }
        }
        self.get_attr_val(obj_val, attr, Some(cache))
    }

    /// スライス式 `begin:end:step` を評価して `Value::Slice` を生成する。
    /// 各境界は int・Index インスタンス・None のいずれかでなければならない。
    pub(crate) fn eval_slice_expr(
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
    pub(crate) fn eval_binop_expr(&mut self, op: &BinOp, left: &Expr, right: &Expr) -> Result<Value, String> {
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
    pub(crate) fn eval_match_expr(&mut self, subject: &Expr, arms: &[MatchArm]) -> Result<Value, String> {
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

}
