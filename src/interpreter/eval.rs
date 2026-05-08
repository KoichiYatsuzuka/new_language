// eval.rs — 式の評価・attr_assign (eval / attr_assign)
//
// `Interpreter::eval` が式（`Expr`）を再帰的にツリーウォークして `Value` を返す。
// 属性への代入（`self.x = v` や `d[k] = v`）は `attr_assign` が担当する。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{BinOp, Expr};

use super::{DictData, Interpreter, TupleData, Value};

impl Interpreter {
    /// 式（`Expr`）を評価して `Value` を返す。インタープリタの式評価のメインエントリポイント。
    ///
    /// 各 `Expr` バリアントを再帰的に処理する:
    /// - リテラル値（Int/Float/Str/Bool/None）はそのまま対応する `Value` に変換
    /// - `Ident`: スコープ検索して変数の値を返す
    /// - `Attr` / `TraitAccess`: インスタンスフィールド・クラス変数・メソッドを順に検索
    /// - `List` / `Tuple` / `Dict`: 各要素を評価してコレクション値を構築
    /// - `Subscript`: 辞書のキールックアップ
    /// - `BinOp`: `and`/`or` は短絡評価、それ以外は `apply_binop` に委譲
    /// - `Call`: テンプレート呼び出し・メソッド呼び出し・組み込み関数・ユーザー定義関数の順に処理
    /// - `TemplateInstantiate`: 単独では使用不可（`Call` の一部として処理する）
    ///
    /// - `expr`: 評価する式の AST ノード
    ///
    /// 戻り値: `Ok(Value)` — 評価結果。`Err(message)` — ランタイムエラー（NameError 等）
    pub fn eval(&mut self, expr: &Expr) -> Result<Value, String> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(f) => Ok(Value::Float(*f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::None => Ok(Value::None),
            Expr::Ident(name) => self
                .get_var(name)
                .map(|v| v.value.clone())
                .ok_or_else(|| format!("NameError: '{name}' is not defined")),
            Expr::TraitAccess { object, trait_name, attr } => {
                // `object::TraitName::attr` 形式のトレイトフィールドアクセス
                let obj_val = self.eval(object)?;
                match obj_val {
                    Value::Instance(inst_rc) => {
                        let inst = inst_rc.borrow();
                        // フィールドは "TraitName::attr" のキーで格納されている
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
            Expr::Attr { object, attr } => {
                // `object.attr` 形式の属性アクセス: インスタンスフィールド → クラス変数 → メソッドの順に検索
                let obj_val = self.eval(object)?;
                match &obj_val {
                    Value::Instance(inst_rc) => {
                        let inst = inst_rc.borrow();
                        // 1. インスタンスフィールドを直接キーで検索
                        if let Some((v, _)) = inst.fields.get(attr.as_str()) {
                            return Ok(v.clone());
                        }
                        // 1b. トレイト名前空間付きフィールドのフォールバック検索（"Trait::attr" 形式）
                        let suffix = format!("::{attr}");
                        if let Some((v, _)) = inst.fields.iter().find_map(|(k, v)| {
                            if k.ends_with(suffix.as_str()) { Some(v) } else { None }
                        }) {
                            return Ok(v.clone());
                        }
                        // 2. const クラス変数を検索
                        if let Some(v) = Self::lookup_class_var(&inst.class, attr) {
                            return Ok(v);
                        }
                        // 3. メソッドを検索（オーバーロードがある場合は OverloadedFn を返す）
                        if let Some(overloads) = inst.class.methods.get(attr.as_str()) {
                            return Ok(if overloads.len() == 1 {
                                Value::Function(overloads[0].clone())
                            } else {
                                Value::OverloadedFn(overloads.clone())
                            });
                        }
                        Err(format!(
                            "AttributeError: '{}' object has no attribute '{attr}'",
                            inst.class.name
                        ))
                    }
                    Value::Class(cls) => {
                        // クラスオブジェクトへのアクセス: クラス変数 → メソッドの順に検索
                        if let Some(v) = Self::lookup_class_var(cls, attr) {
                            return Ok(v);
                        }
                        if let Some(overloads) = cls.methods.get(attr.as_str()) {
                            return Ok(if overloads.len() == 1 {
                                Value::Function(overloads[0].clone())
                            } else {
                                Value::OverloadedFn(overloads.clone())
                            });
                        }
                        Err(format!("AttributeError: class '{}' has no attribute '{attr}'", cls.name))
                    }
                    _ => Err(format!(
                        "AttributeError: '{}' object has no attribute '{attr}'",
                        self.type_name(&obj_val)
                    )),
                }
            }
            Expr::List(items) => {
                // リストリテラル: 各要素を評価して Value::List を構築する
                let mut vals = Vec::new();
                for item in items {
                    vals.push(self.eval(item)?);
                }
                Ok(Value::List(vals))
            }
            Expr::Tuple(exprs) => {
                // タプルリテラル: 各要素を評価し、型名も収集して TupleData を構築する
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
                // 辞書リテラル: 各キー・値ペアを評価して型なし（Any）辞書を構築する
                let mut keys = Vec::new();
                let mut items = Vec::new();
                for (key_expr, val_expr) in pairs {
                    keys.push(self.eval(key_expr)?);
                    items.push(self.eval(val_expr)?);
                }
                Ok(Value::Dict(Rc::new(RefCell::new(DictData {
                    key_type: "Any".to_string(),
                    item_type: "Any".to_string(),
                    keys,
                    items,
                }))))
            }
            Expr::Subscript { object, index } => {
                // `object[index]`: 現在は辞書のキールックアップのみ対応
                let obj = self.eval(object)?;
                let key = self.eval(index)?;
                match obj {
                    Value::Dict(d) => {
                        d.borrow().get(&key).ok_or_else(|| {
                            format!("KeyError: {}", self.display(&key))
                        })
                    }
                    _ => Err(format!(
                        "TypeError: '{}' object is not subscriptable",
                        self.type_name(&obj)
                    )),
                }
            }
            Expr::UnaryOp { op, operand } => {
                // 単項演算子: オペランドを評価して apply_unary に委譲する
                let val = self.eval(operand)?;
                self.apply_unary(op, val)
            }
            Expr::BinOp { op, left, right, .. } => {
                // and / or は短絡評価（左辺の結果によって右辺を評価しない）
                match op {
                    BinOp::And => {
                        let lv = self.eval(left)?;
                        return if !self.is_truthy(&lv) { Ok(lv) } else { self.eval(right) };
                    }
                    BinOp::Or => {
                        let lv = self.eval(left)?;
                        return if self.is_truthy(&lv) { Ok(lv) } else { self.eval(right) };
                    }
                    _ => {}
                }
                // その他の二項演算子: 両辺を評価して apply_binop に委譲する
                let lv = self.eval(left)?;
                let rv = self.eval(right)?;
                self.apply_binop(op, lv, rv)
            }
            Expr::TemplateInstantiate { .. } => {
                // テンプレート式は単独では使用不可（Call の一部として処理される）
                Err("TemplateError: template expression must be immediately called (e.g. `Func[T](args)`)".to_string())
            }
            Expr::Call { func, args } => {
                // テンプレート呼び出し: `expr[T1, T2](args)` 形式
                if let Expr::TemplateInstantiate { base, type_args } = func.as_ref() {
                    let tmpl_val = self.eval(base)?;
                    return self.instantiate_template(tmpl_val, type_args, args);
                }

                // メソッド呼び出し: `obj.method(args)` 形式
                if let Expr::Attr { object, attr } = func.as_ref() {
                    let obj_val = self.eval(object)?;
                    return self.eval_method_call(obj_val, attr, args);
                }

                // 組み込み関数（スコープに格納されていない特別扱い）
                if let Expr::Ident(name) = func.as_ref() {
                    match name.as_str() {
                        "print" => {
                            // すべての引数を display 形式で評価してスペース区切りで出力する
                            let parts: Result<Vec<_>, _> = args.iter()
                                .map(|a| self.eval(a.expr()).map(|v| self.display(&v)))
                                .collect();
                            println!("{}", parts?.join(" "));
                            return Ok(Value::None);
                        }
                        "range" => {
                            // range(stop) / range(start, stop) / range(start, stop, step) をリストに展開する
                            let evaled: Result<Vec<_>, _> =
                                args.iter().map(|a| self.eval(a.expr())).collect();
                            let evaled = evaled?;
                            return match evaled.as_slice() {
                                [Value::Int(stop)] => {
                                    Ok(Value::List((0..*stop).map(Value::Int).collect()))
                                }
                                [Value::Int(start), Value::Int(stop)] => {
                                    Ok(Value::List((*start..*stop).map(Value::Int).collect()))
                                }
                                [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                                    let mut items = Vec::new();
                                    let mut i = *start;
                                    if *step > 0 {
                                        while i < *stop { items.push(Value::Int(i)); i += step; }
                                    } else if *step < 0 {
                                        while i > *stop { items.push(Value::Int(i)); i += step; }
                                    }
                                    Ok(Value::List(items))
                                }
                                _ => Err("TypeError: range() takes 1\u{2013}3 integer arguments".to_string()),
                            };
                        }
                        "len" => {
                            // len(list) / len(str) のみ対応
                            if args.len() != 1 {
                                return Err("TypeError: len() takes exactly one argument".to_string());
                            }
                            let val = self.eval(args[0].expr())?;
                            return match val {
                                Value::List(items) => Ok(Value::Int(items.len() as i64)),
                                Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                                _ => Err(format!("TypeError: object of type '{}' has no len()", self.type_name(&val))),
                            };
                        }
                        _ => {} // ユーザー定義関数の検索へフォールスルー
                    }
                }

                // ユーザー定義関数 / オーバーロード関数 / クラスコンストラクタ / ジェネレータ関数
                // トレースバックフレーム用に関数名を取得（ベストエフォート）
                let call_name = match func.as_ref() {
                    Expr::Ident(n) => n.clone(),
                    _ => "<anonymous>".to_string(),
                };
                let callee = self.eval(func)?;
                match callee {
                    Value::Function(fn_val) => self.exec_fn(fn_val, args, None, &call_name),
                    Value::OverloadedFn(candidates) => {
                        let evaled_args = self.eval_call_args(args)?;
                        self.dispatch_overload_evaled(candidates, evaled_args, None, &call_name)
                    }
                    Value::Class(cls) => self.instantiate(cls, args),
                    Value::GeneratorFn(gen_fn) => self.exec_generator(gen_fn, args, None),
                    Value::TemplateFn(_) | Value::TemplateClass(_) | Value::TemplateGenFn(_) => Err(
                        "TemplateError: template must be called with explicit type arguments (e.g. `Func[T](args)`)".to_string()
                    ),
                    _ => Err(format!("TypeError: '{}' object is not callable", self.type_name(&callee))),
                }
            }
        }
    }

    // --- 属性代入ヘルパー ---

    /// 属性・添字に値を代入する。`AttrAssign` 文と `AttrCompoundAssign` 文から呼ばれる。
    ///
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
        if let Expr::Attr { object, attr } = target {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    let inst_class = inst_rc.borrow().class.clone();
                    if Self::lookup_class_var(&inst_class, attr).is_some() {
                        return Err(format!(
                            "TypeError: cannot assign to class variable '{attr}' (declared const)"
                        ));
                    }
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
                        let is_mutable = inst.class.field_mutability
                            .get(attr.as_str()).copied().unwrap_or(true);
                        inst.fields.insert(attr.clone(), (rhs, is_mutable));
                    }
                    Ok(())
                }
                _ => Err("AttributeError: cannot set attribute on non-instance".to_string()),
            }
        } else if let Expr::TraitAccess { object, trait_name, attr } = target {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    // Trait fields are stored with a namespaced key "TraitName::field"
                    let key = format!("{}::{}", trait_name, attr);
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
            match obj_val {
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
                _ => Err(format!(
                    "TypeError: '{}' object does not support item assignment",
                    self.type_name(&obj_val)
                )),
            }
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
            _ => {
                if let Value::Instance(inst) = val {
                    inst.borrow().class.name == type_name
                } else {
                    false
                }
            }
        }
    }
}
