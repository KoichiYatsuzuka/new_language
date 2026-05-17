// eval.rs — 式の評価・attr_assign (eval / attr_assign)
//
// `Interpreter::eval` が式（`Expr`）を再帰的にツリーウォークして `Value` を返す。
// 属性への代入（`self.x = v` や `d[k] = v`）は `attr_assign` が担当する。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{Accessibility, BinOp, Expr};

use super::{DictData, ExecResult, FileData, FileOpenModeRust, ByteModeRust, GeneratorState, Interpreter, TupleData, Value, Var, RAISE_SENTINEL, BLOCK_YIELDS, LOOP_DEPTH};

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
    fn check_member_access(&self, class: &super::ClassValue, member_key: &str, display_name: &str) -> Result<(), String> {
        let access = class.field_access.get(member_key)
            .or_else(|| class.method_access.get(member_key))
            .cloned()
            .unwrap_or(Accessibility::Public);
        match access {
            Accessibility::Public => Ok(()),
            Accessibility::Private => {
                if let Some(cur) = &self.current_class {
                    if cur.name == class.name { return Ok(()); }
                }
                Err(format!(
                    "AccessError: '{}' is private and cannot be accessed outside '{}'",
                    display_name, class.name
                ))
            }
            Accessibility::Protected => {
                if let Some(cur) = &self.current_class {
                    if cur.name == class.name { return Ok(()); }
                    // subclass: current_class has class.name in its bases
                    if cur.bases.contains(&class.name) { return Ok(()); }
                }
                Err(format!(
                    "AccessError: '{}' is protected and cannot be accessed outside '{}' or its subclasses",
                    display_name, class.name
                ))
            }
        }
    }

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
                .get_val(name)
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
                        let cls = inst.class.clone();
                        // 1. インスタンスフィールドを直接キーで検索
                        if let Some((v, _)) = inst.fields.get(attr.as_str()) {
                            let v = v.clone();
                            drop(inst);
                            self.check_member_access(&cls, attr, attr)?;
                            return Ok(v);
                        }
                        // 1b. トレイト名前空間付きフィールドのフォールバック検索（"Trait::attr" 形式）
                        let suffix = format!("::{attr}");
                        if let Some((full_key, (v, _))) = inst.fields.iter().find(|(k, _)| k.ends_with(suffix.as_str())) {
                            let v = v.clone();
                            let full_key = full_key.clone();
                            drop(inst);
                            self.check_member_access(&cls, &full_key, attr)?;
                            return Ok(v);
                        }
                        // 2. const クラス変数を検索
                        if let Some(v) = Self::lookup_class_var(&cls, attr) {
                            drop(inst);
                            self.check_member_access(&cls, attr, attr)?;
                            return Ok(v);
                        }
                        // 3. メソッドを検索（オーバーロードがある場合は OverloadedFn を返す）
                        if let Some(overloads) = cls.methods.get(attr.as_str()) {
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
                    Value::Namespace(ns) => {
                        // 名前空間（import したモジュール）への属性アクセス
                        ns.members.get(attr.as_str())
                            .cloned()
                            .ok_or_else(|| format!(
                                "AttributeError: module '{}' has no attribute '{attr}'",
                                ns.name
                            ))
                    }
                    Value::PyObject(handle) => {
                        // Python オブジェクトへの属性アクセス: PyO3 経由で getattr を呼ぶ
                        super::py_interop::py_getattr(&handle, attr)
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
                Ok(Value::List(Rc::new(RefCell::new(vals))))
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
                let obj = self.eval(object)?;
                let key = self.eval(index)?;
                self.eval_subscript(obj, key)
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
            Expr::Block { stmts, .. } => {
                // block: 式。block_return で即終了、block_yield でスレッドローカルに値を積みながら継続する。
                // ネストした block: 式を正しく扱うため、BLOCK_YIELDS の前の値を退避して評価後に復元する。
                self.eval_block_expr(stmts)
            }
            Expr::IfExpr { branches, else_body, .. } => {
                // if 式: block_return で値を返す。BLOCK_YIELDS は外側コンテキストを引き継ぐ（透過的）。
                for (cond, body) in branches {
                    let val = self.eval(cond)?;
                    if self.is_truthy(&val) {
                        return self.eval_capture_block_return(body);
                    }
                }
                if let Some(body) = else_body {
                    return self.eval_capture_block_return(body);
                }
                Ok(Value::None)
            }
            Expr::ForExpr { target, iter, body, .. } => {
                // for 式: block_yield でリスト蓄積、block_return で単値返却。
                // break (= block_return None) はループを終了し蓄積リストを返す。
                self.eval_for_expr(target, iter, body)
            }
            Expr::WhileExpr { cond, body, .. } => {
                // while 式: ForExpr と同様。
                self.eval_while_expr(cond, body)
            }
            Expr::MatchExpr { subject, arms, .. } => {
                // match 式: block_return で値を返す。BLOCK_YIELDS は透過的。
                let subject_val = self.eval(subject)?;
                for arm in arms {
                    let matched = match &arm.pattern {
                        crate::ast::MatchPattern::Case(pattern_expr) => {
                            if matches!(pattern_expr, Expr::Ident(n) if n == "_") {
                                true
                            } else {
                                let pv = self.eval(pattern_expr)?;
                                matches!(self.apply_binop(&crate::ast::BinOp::Eq, subject_val.clone(), pv)?, Value::Bool(true))
                            }
                        }
                        crate::ast::MatchPattern::IsType(type_name) => self.value_is_type(&subject_val, type_name),
                    };
                    if matched {
                        return self.eval_capture_block_return(&arm.body);
                    }
                }
                Ok(Value::None)
            }
            Expr::IsType { expr, negated, type_name, .. } => {
                // `x is T` / `x is not T`: 実行時の型判定。value_is_type で確認し Bool を返す。
                let val = self.eval(expr)?;
                let matches = self.value_is_type(&val, type_name);
                Ok(Value::Bool(if *negated { !matches } else { matches }))
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
                                    Ok(Value::List(Rc::new(RefCell::new((0..*stop).map(Value::Int).collect()))))
                                }
                                [Value::Int(start), Value::Int(stop)] => {
                                    Ok(Value::List(Rc::new(RefCell::new((*start..*stop).map(Value::Int).collect()))))
                                }
                                [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                                    let mut items = Vec::new();
                                    let mut i = *start;
                                    if *step > 0 {
                                        while i < *stop { items.push(Value::Int(i)); i += step; }
                                    } else if *step < 0 {
                                        while i > *stop { items.push(Value::Int(i)); i += step; }
                                    }
                                    Ok(Value::List(Rc::new(RefCell::new(items))))
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
                                Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
                                Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                                Value::PyObject(ref handle) => {
                                    super::py_interop::py_len(handle)
                                }
                                _ => Err(format!("TypeError: object of type '{}' has no len()", self.type_name(&val))),
                            };
                        }
                        "open" => {
                            use std::collections::HashMap as HMap;
                            use std::fs::OpenOptions;
                            use std::io::Read as IoRead;
                            let evaled = self.eval_call_args(args)?;
                            let mut kw: HMap<String, Value> = HMap::new();
                            let mut pos: Vec<Value> = Vec::new();
                            for (k, v) in evaled {
                                match k { Some(n) => { kw.insert(n, v); } None => pos.push(v) }
                            }
                            let file_path = extract_path_str(
                                get_arg(&pos, &kw, 0, "file_path")
                                    .ok_or("TypeError: open() missing required argument 'file_path'")?
                            )?;
                            let open_mode_int = extract_enum_int(
                                get_arg(&pos, &kw, 1, "open_mode")
                                    .ok_or("TypeError: open() missing required argument 'open_mode'")?,
                                "enum_item_FileOpenMode",
                            )?;
                            let start_point_int: i64 = get_arg(&pos, &kw, 2, "start_point")
                                .map(|v| extract_enum_int(v, "enum_item_StartPoint"))
                                .transpose()?.unwrap_or(0); // default: top
                            let byte_mode_int: i64 = get_arg(&pos, &kw, 3, "byte_recognizing")
                                .map(|v| extract_enum_int(v, "enum_item_ByteRecognizingMode"))
                                .transpose()?.unwrap_or(1); // default: text
                            let enc_int: i64 = get_arg(&pos, &kw, 4, "encoding")
                                .map(|v| extract_enum_int(v, "enum_item_Encoding"))
                                .transpose()?.unwrap_or(1); // default: UTF_8
                            if enc_int == 3 {
                                return Err("NotImplementedError: Shift-JIS encoding is not yet supported".to_string());
                            }
                            let _exclusion: bool = get_arg(&pos, &kw, 5, "exclusion")
                                .map(|v| match v {
                                    Value::Bool(b) => Ok(*b),
                                    _ => Err("TypeError: open() 'exclusion' must be bool".to_string()),
                                })
                                .transpose()?.unwrap_or(true);

                            let mode = match open_mode_int {
                                0 => FileOpenModeRust::Write,
                                1 => FileOpenModeRust::Rewrite,
                                2 => FileOpenModeRust::Read,
                                3 => FileOpenModeRust::MakeAndWrite,
                                n => return Err(format!("TypeError: invalid FileOpenMode value {n}")),
                            };
                            let byte_mode = if byte_mode_int == 0 { ByteModeRust::Byte } else { ByteModeRust::Text };

                            let std_path = std::path::Path::new(&file_path);
                            if mode == FileOpenModeRust::MakeAndWrite && std_path.exists() {
                                return Err(format!(
                                    "RuntimeError: open() make_and_write: file '{}' already exists",
                                    file_path
                                ));
                            }

                            let (file, content) = match mode {
                                FileOpenModeRust::Read => {
                                    let mut f = OpenOptions::new().read(true).open(std_path)
                                        .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                                    let mut c = Vec::new();
                                    f.read_to_end(&mut c)
                                        .map_err(|e| format!("IOError: cannot read '{}': {e}", file_path))?;
                                    (f, c)
                                }
                                FileOpenModeRust::Write => {
                                    let mut f = OpenOptions::new().read(true).write(true).open(std_path)
                                        .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                                    let mut c = Vec::new();
                                    f.read_to_end(&mut c)
                                        .map_err(|e| format!("IOError: cannot read '{}': {e}", file_path))?;
                                    (f, c)
                                }
                                FileOpenModeRust::Rewrite => {
                                    let f = OpenOptions::new()
                                        .read(true).write(true).create(true).truncate(true)
                                        .open(std_path)
                                        .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                                    (f, Vec::new()) // 内容を空にする
                                }
                                FileOpenModeRust::MakeAndWrite => {
                                    let f = OpenOptions::new()
                                        .read(true).write(true).create_new(true)
                                        .open(std_path)
                                        .map_err(|e| format!("IOError: cannot create '{}': {e}", file_path))?;
                                    (f, Vec::new())
                                }
                            };

                            // BOM 付き UTF-8 の場合は先頭3バイトをスキップ
                            let (content, bom_skip) = if enc_int == 2
                                && content.starts_with(&[0xEF, 0xBB, 0xBF])
                            {
                                (content[3..].to_vec(), 3usize)
                            } else {
                                (content, 0usize)
                            };
                            let _ = bom_skip;

                            let pointer = if start_point_int == 1 { content.len() } else { 0 };

                            let fd = FileData {
                                path: file_path,
                                mode,
                                byte_mode,
                                content,
                                pointer,
                                is_closed: false,
                                file_handle: Some(file),
                            };
                            return Ok(Value::FileObject(Rc::new(RefCell::new(fd))));
                        }
                        "close" => {
                            if args.len() != 1 {
                                return Err("TypeError: close() takes exactly one argument".to_string());
                            }
                            let val = self.eval(args[0].expr())?;
                            return match val {
                                Value::FileObject(fd_rc) => {
                                    fd_rc.borrow_mut().close();
                                    Ok(Value::None)
                                }
                                other => Err(format!(
                                    "TypeError: close() argument must be FileObject, not '{}'",
                                    self.type_name(&other)
                                )),
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
                    Value::PyObject(handle) => {
                        // Python callable を PyO3 経由で呼び出す
                        let evaled_args = self.eval_call_args(args)?;
                        super::py_interop::call_py_object(&handle, &evaled_args)
                    }
                    Value::Instance(_) => {
                        // インスタンスを呼び出す: __call__ メソッドに委譲する
                        self.eval_method_call(callee, "__call__", args)
                    }
                    _ => Err(format!("TypeError: '{}' object is not callable", self.type_name(&callee))),
                }
            }
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
                Ok(ExecResult::BlockReturn(v)) => { block_return_val = Some(v); break 'block_expr; }
                Ok(ExecResult::BlockYield(_)) => {} // スレッドローカル経由で収集済み
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'block_expr;
                }
                Ok(ExecResult::Return(_)) => {
                    early_err = Some("SyntaxError: 'return' inside block expression — use 'block_return'".to_string());
                    break 'block_expr;
                }
                Ok(ExecResult::Break) | Ok(ExecResult::Continue) => {
                    // block: 式の外にループが無い場合; ループ内にいれば外側ループで処理される
                    early_err = Some("SyntaxError: 'break'/'continue' inside block expression is not supported outside a loop".to_string());
                    break 'block_expr;
                }
                Err(e) => { early_err = Some(e); break 'block_expr; }
            }
        }
        self.pop_scope();

        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err { return Err(e); }
        match block_return_val {
            Some(v) => Ok(v),
            None => if yields.is_empty() { Ok(Value::None) } else { Ok(Value::List(Rc::new(RefCell::new(yields)))) },
        }
    }

    /// if / match 式のボディを実行し、BlockReturn シグナルを値として捕捉して返す。
    /// BLOCK_YIELDS は設定しない（透過的 — 外側の for/while/block 式に yield が届く）。
    pub(super) fn eval_capture_block_return(&mut self, stmts: &[crate::ast::Stmt]) -> Result<Value, String> {
        self.push_scope();
        let mut result_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'body: for stmt in stmts {
            match self.exec(stmt) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::BlockReturn(v)) => { result_val = Some(v); break 'body; }
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'body;
                }
                Ok(ExecResult::Return(_)) => {
                    early_err = Some("SyntaxError: 'return' inside block expression — use 'block_return'".to_string());
                    break 'body;
                }
                Ok(other) => {
                    // Break, Continue, BlockYield: 伝播させない（ここでは捕捉できない）
                    // これらが届くのは制御フロー式のネストが正しくない場合
                    // Continue/Break は内側のループに渡すために伝播させる
                    let _ = other; // Normal として継続
                }
                Err(e) => { early_err = Some(e); break 'body; }
            }
        }
        self.pop_scope();
        if let Some(e) = early_err { return Err(e); }
        Ok(result_val.unwrap_or(Value::None))
    }

    /// for 式の実体。BLOCK_YIELDS コンテキストと LOOP_DEPTH を管理し、loop_yield でリスト蓄積、block_return で単値返却。
    pub(super) fn eval_for_expr(&mut self, target: &str, iter_expr: &crate::ast::Expr, body: &[crate::ast::Stmt]) -> Result<Value, String> {
        let iter_val = self.eval(iter_expr)?;
        let generator = match iter_val {
            Value::List(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items.borrow().clone(), index: 0 }))),
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState { values: chars, index: 0 })))
            }
            Value::Generator(_) => iter_val,
            Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[])?,
            Value::PyObject(ref handle) => {
                let items = super::py_interop::py_collect_iter(handle)?;
                Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items, index: 0 })))
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
                        Ok(ExecResult::Break) | Ok(ExecResult::BlockReturn(Value::None)) => break 'for_loop,
                        Ok(ExecResult::BlockReturn(v)) => { block_return_val = Some(v); break 'for_loop; }
                        Ok(ExecResult::Raise(raised)) => {
                            self.current_exception = Some(raised);
                            early_err = Some(RAISE_SENTINEL.to_string());
                            break 'for_loop;
                        }
                        Ok(ExecResult::Return(v)) => { block_return_val = Some(v); break 'for_loop; } // shouldn't happen
                        Ok(ExecResult::BlockYield(_)) => {}
                        Err(e) => { early_err = Some(e); break 'for_loop; }
                    }
                }
                Err(ref e) if e.starts_with("EndOfIteration") => break,
                Err(e) => { early_err = Some(e); break; }
            }
        }

        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err { return Err(e); }
        match block_return_val {
            Some(v) => Ok(v),
            None => if yields.is_empty() { Ok(Value::None) } else { Ok(Value::List(Rc::new(RefCell::new(yields)))) },
        }
    }

    /// while 式の実体。for 式と同様に BLOCK_YIELDS と LOOP_DEPTH を管理する。
    pub(super) fn eval_while_expr(&mut self, cond_expr: &crate::ast::Expr, body: &[crate::ast::Stmt]) -> Result<Value, String> {
        let saved = BLOCK_YIELDS.with(|y| y.borrow_mut().take());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);

        let mut block_return_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'while_loop: loop {
            let cond_val = match self.eval(cond_expr) {
                Ok(v) => v,
                Err(e) => { early_err = Some(e); break; }
            };
            if !self.is_truthy(&cond_val) { break; }

            match self.exec_scoped_block(body) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::Continue) => continue,
                Ok(ExecResult::Break) | Ok(ExecResult::BlockReturn(Value::None)) => break 'while_loop,
                Ok(ExecResult::BlockReturn(v)) => { block_return_val = Some(v); break 'while_loop; }
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'while_loop;
                }
                Ok(other) => { let _ = other; }
                Err(e) => { early_err = Some(e); break; }
            }
        }

        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err { return Err(e); }
        match block_return_val {
            Some(v) => Ok(v),
            None => if yields.is_empty() { Ok(Value::None) } else { Ok(Value::List(Rc::new(RefCell::new(yields)))) },
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
            _ => {
                if let Value::Instance(inst) = val {
                    inst.borrow().class.name == type_name
                } else {
                    false
                }
            }
        }
    }

    /// `obj[key]` の評価。リスト・文字列・辞書・PyObject・インスタンスに対応する。
    pub(super) fn eval_subscript(&mut self, obj: Value, key: Value) -> Result<Value, String> {
        match obj {
            Value::List(items) => {
                let idx = match &key {
                    Value::Int(i) => *i,
                    _ => return Err(format!(
                        "TypeError: list indices must be integers, not '{}'",
                        self.type_name(&key)
                    )),
                };
                let borrowed = items.borrow();
                let len = borrowed.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: list index {} out of range", idx));
                }
                Ok(borrowed[actual as usize].clone())
            }
            Value::Str(s) => {
                let idx = match &key {
                    Value::Int(i) => *i,
                    _ => return Err(format!(
                        "TypeError: string indices must be integers, not '{}'",
                        self.type_name(&key)
                    )),
                };
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: string index {} out of range", idx));
                }
                Ok(Value::Str(chars[actual as usize].to_string()))
            }
            Value::Tuple(td) => {
                let idx = match &key {
                    Value::Int(i) => *i,
                    _ => return Err(format!(
                        "TypeError: tuple indices must be integers, not '{}'",
                        self.type_name(&key)
                    )),
                };
                let vals = td.all_values();
                let len = vals.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: tuple index {} out of range", idx));
                }
                Ok(vals[actual as usize].clone())
            }
            Value::Dict(d) => {
                d.borrow().get(&key).ok_or_else(|| format!("KeyError: {}", self.display(&key)))
            }
            Value::Instance(_) => {
                self.eval_method_call_evaled(obj, "__getitem__", vec![(None, key)])
            }
            Value::PyObject(ref handle) => {
                super::py_interop::py_getitem(handle, &key)
            }
            _ => Err(format!(
                "TypeError: '{}' object is not subscriptable",
                self.type_name(&obj)
            )),
        }
    }

    /// `obj[key] = rhs` の実行。リスト・辞書・PyObject・インスタンスに対応する。
    pub(super) fn eval_setitem(&mut self, obj: Value, key: Value, rhs: Value) -> Result<(), String> {
        match obj {
            Value::List(items) => {
                let idx = match &key {
                    Value::Int(i) => *i,
                    _ => return Err(format!(
                        "TypeError: list indices must be integers, not '{}'",
                        self.type_name(&key)
                    )),
                };
                let mut borrowed = items.borrow_mut();
                let len = borrowed.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: list assignment index {} out of range", idx));
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
            Value::PyObject(ref handle) => {
                super::py_interop::py_setitem(handle, &key, &rhs)
            }
            _ => Err(format!(
                "TypeError: '{}' object does not support item assignment",
                self.type_name(&obj)
            )),
        }
    }
}
