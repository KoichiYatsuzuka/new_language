// ops/typecheck.rs — 型・真偽値の判定: is_truthy / type_name / value_matches_type_ann / check_block_return_type / value_is_type / type_name_of。

use crate::interpreter::{Interpreter, Value};

/// `"list[T]"` からアイテム型 `"T"` を取り出す。`"list"` や他の型は `None` を返す。
fn extract_list_elem_type(ann: &str) -> Option<&str> {
    let inner = ann.strip_prefix("list[")?.strip_suffix(']')?;
    Some(inner.trim())
}

/// 値のランタイム型名（エラーメッセージ用）。**`Interpreter` を持たない場所でも使う**ため
/// 自由関数にしてある（`DictData::reject_key` がキーの型名を出すのに要る・B1-a）。
/// `Interpreter::type_name` はこれに委譲するだけ。
pub(crate) fn runtime_type_name(val: &Value) -> &'static str {
    match val {
        Value::Int(_) => "int",
        Value::UInt(_) => "uint",
        Value::Float(_) => "float",
        Value::Complex(_, _) => "complex",
        Value::Str(_) => "str",
        Value::Bool(_) => "bool",
        Value::None => "NoneType",
        Value::Undefined => "Undefined",
        Value::List(_) => "list",
        Value::FrozenList { .. } => "fixed_list",
        Value::Function(_) | Value::OverloadedFn(_) => "function",
        Value::Class(_) | Value::Type(_) => "type",
        Value::Trait(_) => "trait",
        Value::Protocol(_) => "protocol",
        Value::Instance(_) => "object",
        Value::TemplateFn(_) | Value::TemplateClass(_) => "template",
        Value::GeneratorFn(_) | Value::TemplateGenFn(_) => "gen_function",
        Value::Generator(_) => "generator",
        Value::Dict(_) => "dict",
        Value::Tuple(_) => "tuple",
        Value::Set(_) => "set",
        Value::Namespace(_) => "module",
        Value::PyObject(_) => "object",
        Value::FileObject(_) => "FileObject",
        Value::NativeFunction(_) => "function",
        Value::Slice(_) => "slice",
        Value::AsyncManager(_) => "AsyncManager",
        Value::AsyncStatusVal(_) => "Async",
        Value::Signal(_) => "Signal",
        Value::EventLoop(_) => "EventLoop",
        Value::CsObject(o) => {
            let _ = o;
            "cs_object"
        }
        Value::JsProcFn(_) => "function",
        Value::ResultVal { ok, .. } => {
            if *ok { "Ok" } else { "Err" }
        }
    }
}

/// **組み込み型名 → 値が適合するか**（タスク 6.1）。
///
/// # なぜ 1 本にまとめたのか
///
/// 実行時の適合判定は**独立に 3 本**あり、同じ型名について答えが割れていた。
/// `uint` で実測した食い違い（`let x: uint = 5` の実体は `Value::Int`）:
///
/// | 経路 | 述語 | 結果 |
/// |---|---|---|
/// | `x is uint` | `value_is_type` | **False**（`Value::UInt` を要求） |
/// | `block ->uint: block_return 5` | `value_matches_type_ann` | **TypeError**（拒否） |
/// | フィールド代入 | `TypeTag` ＋ A-3 の相互許容 | 通る |
///
/// ⇒ **同じ値・同じ型名なのに地点で答えが違う**状態だった。
///
/// # `uint` と `int` は相互に許容する
///
/// ⚠⚠ `Value::UInt` が生まれるのは**ハンドル値と `uint()` 変換だけ**で、
/// `let x: uint = 5` も `f(v: uint)` も値としては `Value::Int` を束縛する。
/// `uint` の位置で `Value::UInt` だけを要求すると、実バグを塞がずに**正常なコードが落ちる**。
/// ⇒ 整数族は通す（`str` / `bool` の混入は弾く）。A-3 が `store_field` に入れた規則と同じ。
///
/// 戻り値 `None` は「**この表が知らない名前**」＝クラス・trait・protocol・容器の
/// 型引数つき表記など。呼び出し側がそれぞれの解決へ落とす。
pub(crate) fn primitive_ann_matches(ann: &str, val: &Value) -> Option<bool> {
    // C ABI 型（int32 等）は基底型（int/float）の別名として扱う。
    let ann = crate::ast::c_abi_base_type(ann).unwrap_or(ann);
    let r = match ann {
        "Any" => true,
        "None" => matches!(val, Value::None),
        "Undefined" => matches!(val, Value::Undefined),
        // ⚠ 整数族（上の doc）。
        "int" | "uint" => matches!(val, Value::Int(_) | Value::UInt(_)),
        "float" => matches!(val, Value::Float(_)),
        "complex" => matches!(val, Value::Complex(_, _)),
        "str" => matches!(val, Value::Str(_)),
        "bool" => matches!(val, Value::Bool(_)),
        "list" => matches!(val, Value::List(_)),
        "fixed_list" => matches!(val, Value::FrozenList { .. }),
        "list_like" => matches!(val, Value::List(_) | Value::FrozenList { .. }),
        "dict" => matches!(val, Value::Dict(_)),
        "set" => matches!(val, Value::Set(_)),
        "tuple" => matches!(val, Value::Tuple(_)),
        "slice" => matches!(val, Value::Slice(_)),
        // ⚠ `__call__` を持つインスタンス・クラスも「呼べる」ので `function`。
        //    以前は `value_matches_type_ann` が `JsProcFn` を含み `value_is_type` が
        //    含まないという**もう 1 つの食い違い**があった。
        "function" => match val {
            Value::Function(_)
            | Value::OverloadedFn(_)
            | Value::GeneratorFn(_)
            | Value::NativeFunction(_)
            | Value::JsProcFn(_) => true,
            Value::Instance(inst) => inst.borrow().class.methods.contains_key("__call__"),
            Value::Class(cls) => cls.methods.contains_key("__call__"),
            _ => false,
        },
        // 型引数つきの容器表記は外側だけ見る（要素型は静的検査の担当）。
        _ if ann.starts_with("list[") => matches!(val, Value::List(_)),
        _ if ann.starts_with("fixed_list[") => matches!(val, Value::FrozenList { .. }),
        _ if ann.starts_with("list_like[") => {
            matches!(val, Value::List(_) | Value::FrozenList { .. })
        }
        _ if ann.starts_with("dict[") => matches!(val, Value::Dict(_)),
        _ if ann.starts_with("set[") => matches!(val, Value::Set(_)),
        _ if ann.starts_with("tuple[") => matches!(val, Value::Tuple(_)),
        _ => return None,
    };
    Some(r)
}

impl Interpreter {
    /// 値の真偽判定を行う。
    ///
    /// Python ライクなルール:
    /// - `Bool` → そのまま
    /// - `Int` → `0` なら偽、それ以外は真
    /// - `Float` → `0.0` なら偽、それ以外は真
    /// - `Str` → 空文字列なら偽、それ以外は真
    /// - `None` → 偽
    /// - `List` → 空リストなら偽、非空なら真
    /// - `Dict` → 空辞書なら偽、非空なら真
    /// - `Tuple` → 空タプルなら偽、非空なら真
    /// - 関数・クラス・インスタンス等 → 常に真
    ///
    /// - `val`: 真偽を判定する値
    ///
    /// 戻り値: `true` または `false`
    pub(crate) fn is_truthy(&self, val: &Value) -> bool {
        match val {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::UInt(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Complex(re, im) => *re != 0.0 || *im != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
            Value::Undefined => false,
            Value::List(items) => !items.borrow().is_empty(),
            Value::FrozenList { state, .. } => state.borrow().len > 0,
            Value::Dict(d) => !d.borrow().is_empty(),
            Value::Tuple(t) => !t.is_empty(),
            Value::Set(s) => !s.borrow().is_empty(),
            // 関数・クラス・インスタンス・ジェネレータ・名前空間・Python オブジェクト等は常に真
            Value::Function(_)
            | Value::OverloadedFn(_)
            | Value::Class(_)
            | Value::Instance(_)
            | Value::Type(_)
            | Value::Trait(_)
            | Value::Protocol(_)
            | Value::TemplateFn(_)
            | Value::TemplateClass(_)
            | Value::GeneratorFn(_)
            | Value::TemplateGenFn(_)
            | Value::Generator(_)
            | Value::Namespace(_)
            | Value::PyObject(_)
            | Value::FileObject(_)
            | Value::NativeFunction(_)
            | Value::Slice(_)
            | Value::AsyncManager(_)
            | Value::AsyncStatusVal(_)
            | Value::Signal(_)
            | Value::EventLoop(_)
            | Value::CsObject(_)
            | Value::JsProcFn(_)
            | Value::ResultVal { .. } => true,
        }
    }

    /// 値のランタイム型名を文字列として返す（エラーメッセージや型検査に使用）。
    ///
    /// - `val`: 型名を取得する値
    ///
    /// 戻り値: `"int"`, `"str"`, `"list"`, `"object"` 等の静的文字列
    pub(crate) fn type_name(&self, val: &Value) -> &'static str {
        runtime_type_name(val)
    }

    /// ランタイム値が型アノテーション文字列に一致するかを判定する（block_return/loop_yield の型チェック用）。
    ///
    /// - `Any` は常に true。
    /// - `list[T]`, `dict[K,V]` 等はアウター型のみチェック（`list` として扱う）。
    /// - `Optional[T]` / `Option[T]` は None またはインナー型を受け入れる。
    /// - `Union[T,U,...]` は各候補のいずれかにマッチすれば true。
    /// - クラス名・トレイト名は `value_is_type` に委譲する。
    pub(crate) fn value_matches_type_ann(&self, val: &Value, ann: &str) -> bool {
        // C ABI 型（int32 等）は基底型（int/float）の別名として扱う
        let ann = crate::ast::c_abi_base_type(ann).unwrap_or(ann);
        // ⚠⚠ 組み込み型名は**唯一の表**で判定する（タスク 6.1）。ここに arm を足すと
        //    `value_is_type` / `TypeTag` とずれるので、足すなら `primitive_ann_matches` へ。
        if let Some(r) = primitive_ann_matches(ann, val) {
            return r;
        }
        match ann {
            _ if ann.starts_with("Optional[") || ann.starts_with("Option[") => {
                if matches!(val, Value::None) {
                    return true;
                }
                let inner_start = ann.find('[').map_or(ann.len(), |i| i + 1);
                let inner = ann[inner_start..].trim_end_matches(']');
                self.value_matches_type_ann(val, inner.trim())
            }
            _ if ann.starts_with("Union[") => {
                let inner = ann[6..].trim_end_matches(']');
                inner
                    .split(',')
                    .any(|t| self.value_matches_type_ann(val, t.trim()))
            }
            _ => self.value_is_type(val, ann),
        }
    }

    /// `block_return` の値の型をアノテーション文字列に対してチェックする。
    /// 不一致の場合は `Err(TypeError: ...)` を返す。
    pub(crate) fn check_block_return_type(&self, val: &Value, ann: &str) -> Result<(), String> {
        if self.value_matches_type_ann(val, ann) {
            Ok(())
        } else {
            Err(format!(
                "TypeError: block_return value has type '{}', but '{}' was expected",
                self.type_name(val),
                ann
            ))
        }
    }

    /// `loop_yield` の値の型を、for/while 式の `->list[T]` アノテーションに対して検査する（#35）。
    ///
    /// `ann` が `list[T]` の形でなければ検査しない（`Ok(())`）。
    /// ⚠ **`Op::CheckLoopYield` の唯一の実装**（ツリーウォークの `exec_loop_yield` は #33 で削除）。
    /// メッセージが 1 文字でもずれると off/on 比較が割れる。
    pub(crate) fn check_loop_yield_type(&self, val: &Value, ann: &str) -> Result<(), String> {
        let Some(elem_type) = extract_list_elem_type(ann) else {
            return Ok(());
        };
        if self.value_matches_type_ann(val, elem_type) {
            return Ok(());
        }
        Err(format!(
            "TypeError: loop_yield value has type '{}', but element type '{}' was expected (from ->{})",
            self.type_name(val),
            elem_type,
            ann
        ))
    }

    /// 値が指定した型名に一致するかを判定する（`is` 型ガードのランタイム検査）。
    ///
    /// - プリミティブ型: `type_name` が `"int"`, `"float"` 等と一致するか確認する。
    /// - インスタンス: クラス名または `bases`（実装 trait・基底クラス）に含まれるか確認する。
    /// - `None` 値: `type_name == "None"` の場合のみ `true`。
    pub(crate) fn value_is_type(&self, val: &Value, type_name: &str) -> bool {
        // ⚠⚠ 組み込み型名は**唯一の表**で判定する（タスク 6.1）。以前はここに 2 本目の
        //    写しがあり、`uint`（`Value::UInt` だけを要求）・`function`（`JsProcFn` を
        //    含まない）・`fixed_list` / `list_like` の扱いが `value_matches_type_ann` と
        //    食い違っていた。
        // ⚠ protocol 名がプリミティブ名と衝突することは無い（大文字始まり）ので順序は自由だが、
        //   クラス名より**先**に表を引く。
        if let Some(r) = primitive_ann_matches(type_name, val) {
            return r;
        }
        // Protocol の実行時チェック: 必須メンバー名が全て存在するか確認する
        if let Some(required) = self.protocol_required_members.get(type_name) {
            if let Value::Instance(inst_rc) = val {
                let inst = inst_rc.borrow();
                return required
                    .iter()
                    .all(|m| inst.class.field_index.contains_key(m.as_str()) || inst.class.methods.contains_key(m));
            }
            return false;
        }
        // ここへ来るのは**組み込みでない名前**だけ（クラス・trait・`FileObject` など）。
        match val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                inst.class.name == type_name || inst.class.bases.contains(&type_name.to_string())
            }
            Value::Class(cls) => cls.name == type_name,
            Value::FileObject(_) => type_name == "FileObject",
            _ => false,
        }
    }

    /// `mustbe` エラーメッセージ用: 値のランタイム型名を返す。
    pub(crate) fn type_name_of(&self, val: &Value) -> String {
        match val {
            Value::Int(_) => "int".to_string(),
            Value::UInt(_) => "uint".to_string(),
            Value::Float(_) => "float".to_string(),
            Value::Complex(_, _) => "complex".to_string(),
            Value::Str(_) => "str".to_string(),
            Value::Bool(_) => "bool".to_string(),
            Value::None => "None".to_string(),
            Value::Undefined => "Undefined".to_string(),
            Value::List(_) => "list".to_string(),
            Value::FrozenList { .. } => "fixed_list".to_string(),
            Value::Dict(_) => "dict".to_string(),
            Value::Set(_) => "set".to_string(),
            Value::Tuple(_) => "tuple".to_string(),
            Value::Function(f) => format!("function({})", f.name),
            Value::OverloadedFn(_) => "function(overloaded)".to_string(),
            Value::GeneratorFn(g) => format!("generator_fn({})", g.name),
            Value::NativeFunction(n) => format!("function({})", n.fn_name),
            Value::Class(c) => format!("class({})", c.name),
            Value::Instance(i) => i.borrow().class.name.clone(),
            Value::Slice(_) => "slice".to_string(),
            _ => "unknown".to_string(),
        }
    }

}
