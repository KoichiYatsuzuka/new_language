// ops/typecheck.rs — 型・真偽値の判定: is_truthy / type_name / value_matches_type_ann / check_block_return_type / value_is_type / type_name_of。

use crate::interpreter::{Interpreter, Value};

/// `"list[T]"` からアイテム型 `"T"` を取り出す。`"list"` や他の型は `None` を返す。
fn extract_list_elem_type(ann: &str) -> Option<&str> {
    let inner = ann.strip_prefix("list[")?.strip_suffix(']')?;
    Some(inner.trim())
}

/// 外部言語から受け取った値の型名（タプルの要素型名・エラーメッセージ用）。インスタンスはクラス名、`None` は
/// `NoneType`、それ以外は [`runtime_type_name`]。Python（2-6）と cs-proc / js-proc（5-2）が使う。
/// ⚠ 2-6 より前の Python の境界は表に無い型を 'unknown' と出していた。
#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub(crate) fn element_type_name(val: &Value) -> String {
    match val {
        Value::None => "NoneType".to_string(),
        Value::Instance(inst) => inst.borrow().class.name.clone(),
        Value::CsObject(o) => o.class_name.clone(),
        other => runtime_type_name(other).to_string(),
    }
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
        Value::Bytes(_) => "bytes",
        Value::Bool(_) => "bool",
        Value::None => "NoneType",
        Value::Undefined => "Undefined",
        // 展開時にしか存在しない AST 断片（設計書 §1.2）。
        Value::Code(_) => "Code",
        // ⚠ 種別ごとに違う型名を返す（`meta_class` と `meta_member` は別の型）。
        Value::Meta(m) => match m.kind {
            crate::type_check::MetaKind::Instance => "meta_instance",
            crate::type_check::MetaKind::Function => "meta_function",
            crate::type_check::MetaKind::Class => "meta_class",
            crate::type_check::MetaKind::Member => "meta_member",
        },
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
        Value::FrozenSet(_) => "frozenset",
        Value::Namespace(_) => "module",
        Value::PyObject(_) => "object",
        Value::FileObject(_) => "FileObject",
        Value::NativeFunction(_) => "function",
        Value::Ptr(_) => "Ptr",
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
        // すべての値は `object` の派生（タスク 6-10・CPython と同じ）。`x is object` / 注釈の `object` は何でも受ける。
        "object" => true,
        "None" => matches!(val, Value::None),
        "Undefined" => matches!(val, Value::Undefined),
        // ⚠ 整数族（上の doc）。
        "int" | "uint" => matches!(val, Value::Int(_) | Value::UInt(_)),
        "float" => matches!(val, Value::Float(_)),
        "complex" => matches!(val, Value::Complex(_, _)),
        "str" => matches!(val, Value::Str(_)),
        "bytes" => matches!(val, Value::Bytes(_)),
        "bool" => matches!(val, Value::Bool(_)),
        "list" => matches!(val, Value::List(_)),
        "fixed_list" => matches!(val, Value::FrozenList { .. }),
        "list_like" => matches!(val, Value::List(_) | Value::FrozenList { .. }),
        "dict" => matches!(val, Value::Dict(_)),
        "set" => matches!(val, Value::Set(_)),
        "frozenset" => matches!(val, Value::FrozenSet(_)),
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
        // `Ptr` / `Ptr[T]` は要素型まで見る（3-3・`ptr_ann_matches` の doc）
        _ if ann == "Ptr" || ann.starts_with("Ptr[") => {
            crate::interpreter::value::ptr_ann_matches(ann, val).unwrap_or(false)
        }
        // 型引数つきの容器表記は外側だけ見る（要素型は静的検査の担当）。
        _ if ann.starts_with("list[") => matches!(val, Value::List(_)),
        _ if ann.starts_with("fixed_list[") => matches!(val, Value::FrozenList { .. }),
        _ if ann.starts_with("list_like[") => {
            matches!(val, Value::List(_) | Value::FrozenList { .. })
        }
        _ if ann.starts_with("dict[") => matches!(val, Value::Dict(_)),
        _ if ann.starts_with("set[") => matches!(val, Value::Set(_)),
        _ if ann.starts_with("frozenset[") => matches!(val, Value::FrozenSet(_)),
        _ if ann.starts_with("tuple[") => matches!(val, Value::Tuple(_)),
        // 中身が CPython の list / dict / set（pandas_import_plan.md の D-12・2-7）。ほかの容器と同じく外側だけ見る（要素は
        // 静的検査と、写して受け取るときの照合の担当）。
        _ if ann == "py_list" || ann.starts_with("py_list[") => py_collection_is(val, "list"),
        _ if ann == "py_dict" || ann.starts_with("py_dict[") => py_collection_is(val, "dict"),
        _ if ann == "py_set" || ann.starts_with("py_set[") => py_collection_is(val, "set"),
        // ジェネレータも外側だけ見る（フェーズ10 10-7 で `generator[T]` を注釈に書けるようにした）。
        "generator" => matches!(val, Value::Generator(_)),
        _ if ann.starts_with("generator[") => matches!(val, Value::Generator(_)),
        _ => return None,
    };
    Some(r)
}

/// `val` が CPython の `kind`（`list` / `dict` / `set`）か（`py_list` などの型の照合・D-12）。
fn py_collection_is(val: &Value, kind: &str) -> bool {
    match val {
        Value::PyObject(h) => crate::interpreter::py_interop::py_collection_kind(h) == Some(kind),
        _ => false,
    }
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
            Value::Bytes(b) => !b.is_empty(),
            Value::None => false,
            Value::Undefined => false,
            // 空の `Code`（設計書 §1.2 の B-2）は偽。`if frag:` で「何も出さない」を書ける。
            Value::Code(lines) => !lines.is_empty(),
            // メタ情報は常に真（対象が存在するから値がある）。
            Value::Meta(_) => true,
            Value::List(items) => !items.borrow().is_empty(),
            Value::FrozenList { state, .. } => state.borrow().len > 0,
            Value::Dict(d) => !d.borrow().is_empty(),
            Value::Tuple(t) => !t.is_empty(),
            Value::Set(s) => !s.borrow().is_empty(),
            Value::FrozenSet(s) => !s.is_empty(),
            // ⚠ CPython の物の真偽は CPython に尋ねる（`__bool__` / `__len__`・投げうる）ので `eval_truthy` が先に
            //   受け持つ（2-2）。ここは誤りを返せないので、届いたら真（`__bool__` の無い物と同じ）。届く呼び手は無い
            //   （`apply_unary` は `apply_unary_dyn` からだけ呼ばれ、そちらが CPython の物を先に受ける）。
            Value::PyObject(_) => true,
            // 関数・クラス・インスタンス・ジェネレータ・名前空間等は常に真
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
            | Value::FileObject(_)
            | Value::NativeFunction(_)
            // ⚠ NULL の `Ptr` は無い（NULL は `None`・D-8）ので常に真。解放済みでも真（使えば誤り）
            | Value::Ptr(_)
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

    /// 組み込みの値の種類（protocol の型テストが組み込みの表 `builtin_members` を引く・pandas_import_plan.md の 6-6）。
    /// 組み込みの値でない物（インスタンス・関数・CPython の物など）は `None`。
    ///
    /// ⚠ `match` は網羅（`_` を書かない）。値の種類を足したら、ここで組み込みの値かを決めること（静的な側は
    ///   `builtin_members::shape_of`）。
    pub(crate) fn builtin_kind_of_value(val: &Value) -> Option<crate::type_check::builtin_members::BuiltinKind> {
        use crate::type_check::builtin_members::BuiltinKind as K;
        Some(match val {
            Value::Int(_) | Value::UInt(_) => K::Int,
            Value::Bool(_) => K::Bool,
            Value::Float(_) => K::Float,
            Value::Complex(..) => K::Complex,
            Value::Str(_) => K::Str,
            Value::Bytes(_) => K::Bytes,
            Value::None => K::NoneValue,
            Value::List(_) | Value::FrozenList { .. } => K::List,
            Value::Tuple(_) => K::Tuple,
            Value::Dict(_) => K::Dict,
            Value::Set(_) => K::Set,
            Value::FrozenSet(_) => K::FrozenSet,
            Value::Generator(_) => K::Generator,
            Value::Undefined
            | Value::Code(_)
            | Value::Meta(_)
            | Value::Function(_)
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
            | Value::Namespace(_)
            | Value::PyObject(_)
            | Value::FileObject(_)
            | Value::NativeFunction(_)
            | Value::Ptr(_)
            | Value::Slice(_)
            | Value::AsyncManager(_)
            | Value::AsyncStatusVal(_)
            | Value::Signal(_)
            | Value::EventLoop(_)
            | Value::CsObject(_)
            | Value::JsProcFn(_)
            | Value::ResultVal { .. } => return None,
        })
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
        // ⚠ 名前が今 protocol 以外（利用者の `class Sized` / `enum Sized` が組み込みの protocol を隠した・6-7）に
        //   束縛されていれば、protocol としては見ない（下のクラスの判定へ）。
        let shadowed = || !matches!(self.get_val(type_name), Some(Value::Protocol(_)) | None);
        if let Some(required) = self.protocol_required_members.get(type_name).filter(|_| !shadowed()) {
            return match val {
                Value::Instance(inst_rc) => {
                    let inst = inst_rc.borrow();
                    required
                        .iter()
                        .all(|m| inst.class.field_index.contains_key(m.as_str()) || inst.class.methods.contains_key(m))
                }
                // CPython の物は CPython 3.12 の `isinstance(x, Protocol)` と同じく、属性を静的に引く（6-6）
                Value::PyObject(h) => {
                    required.iter().all(|m| crate::interpreter::py_interop::py_has_static_attr(h, m))
                }
                // 組み込みの値は組み込みの表で見る（`[1, 2] is HasLen` は真・6-6）。⚠ 以前はいつも偽だった
                _ => match Self::builtin_kind_of_value(val) {
                    Some(kind) => {
                        required.iter().all(|m| crate::type_check::builtin_members::has_member(kind, m))
                    }
                    None => false,
                },
            };
        }
        // ここへ来るのは**組み込みでない名前**だけ（クラス・trait・`FileObject` など）。
        match val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                // ⚠ 単相化したクラスは `name` がテンプレートの名前（`Stack`）なので、`is Stack` は
                //   どの具体化にも真、`is Stack[int]` は具体化の名前で見る（フェーズ10 10-10）。
                // 基底・`Exception`（すべての例外の基底・10-19）は `ClassValue::is_a` で見る。
                if inst.class.is_a(type_name) || inst.class.instance_name.as_deref() == Some(type_name) {
                    // ⚠ 自分の名前で一致したときは、その名前を今の場所で引いたクラスと同じクラスか見る（known_bugs.md #30・
                    //   `ar_method_guard` と同じ照合）。実行時のクラスの名前はモジュールで修飾しないので、名前だけだと
                    //   モジュールの `V` と本体の同じ名前の `V` が区別できず、`x is V` / `mustbe V` / `is V:` の腕 /
                    //   `except V` が別のクラスを通していた。基底（`is LookupError`）・`Exception` での一致は名前のまま。
                    let own = inst.class.name == type_name || inst.class.instance_name.as_deref() == Some(type_name);
                    if !own || !self.names_another_class(&inst.class, type_name) {
                        return true;
                    }
                    return inst.class.bases.iter().any(|b| b == type_name);
                }
                // モジュールのクラス（`tags.Tag`・10-8）。
                if !type_name.contains('.') {
                    return false;
                }
                if Self::qualified_class_matches(&inst.class, type_name) {
                    return true;
                }
                // 書いたとおりの別名（`t.Tag`）は名前空間を引いて、同じクラスか・その派生か見る。
                // ⚠ enum のメンバーのクラスは、引いた `Name` クラス（メンバーを持つ方）とは別のクラス
                //   （`build_enum_classes`）。`Self::same_class` が同じ enum かで見る。
                drop(inst);
                let target = match self.dotted_value(type_name) {
                    Some(Value::Class(c)) => c,
                    // 別名を通したテンプレート（`x is m.Box`）は、どの具体化にも真（素の `is Box` と同じ・known_bugs.md #39）。
                    // ⚠ 以前は `dotted_class` がテンプレートを返さず、いつも偽だった。
                    Some(Value::TemplateClass(t)) => return self.is_instantiation_of(&inst_rc.borrow().class, &t),
                    _ => return false,
                };
                if Self::same_class(&inst_rc.borrow().class, &target) {
                    return true;
                }
                // ⚠ 基底（Python のクラスの継承・`d is z.Animal`）。祖先は修飾名で `bases` に載っている
                //   （`exec_class_def`・python_builtins_plan.md の 6 節）。以前は同じクラスしか見ず、派生は偽だった。
                let qualified = match target.module_name.as_deref() {
                    Some(m) => format!("{m}.{}", target.name),
                    None => target.name.clone(),
                };
                inst_rc.borrow().class.is_a(&qualified)
            }
            Value::Class(cls) => cls.name == type_name,
            Value::FileObject(_) => type_name == "FileObject",
            // C# のオブジェクト（cs-dll / cs-proc）。⚠ 以前はこのアームが無く、`x is Person` / `mustbe Person` /
            //   `case Person:` がどれも偽だった。型スタブが `Option[Person]` を返すようになり（nullable 注釈・
            //   ffi_value_exchange_plan.md 2-1）、絞り込みに要る。
            // Python のオブジェクト（Python のクラスのインスタンス・ffi_value_exchange_plan.md 7-27）: Python の `isinstance` と
            // 同じく、クラスとその基底（MRO）の名前で判定する。型をモジュールで修飾して書いたとき（`pc.Box`）は、モジュールの
            // 名前空間の名前とクラスの `__module__` も一致すること（別のモジュールの同名のクラスを取り違えない）
            #[cfg(feature = "native")]
            Value::PyObject(h) => {
                let (qualifier, bare) = match type_name.rsplit_once('.') {
                    Some((q, b)) => (Some(q), b),
                    None => (None, type_name),
                };
                let ns = qualifier.and_then(|q| match self.dotted_value(q) {
                    Some(Value::Namespace(ns)) => Some(ns),
                    _ => None,
                });
                // 名前空間の名前が CPython のクラスを指すなら、CPython の `isinstance` で判定する（pandas_import_plan.md の
                // 1-3）。⚠ 名前と `__module__` の照合だと、再エクスポートしたクラス（`pk.Frame` の `__module__` は定義元の
                // `pk.frames`）が偽になっていた
                if let Some(Value::PyObject(cls)) = ns.as_ref().and_then(|ns| self.namespace_member(ns, bare)) {
                    if let Some(r) = crate::interpreter::py_interop::py_isinstance(h, &cls) {
                        return r;
                    }
                }
                let module = ns.map(|ns| ns.name.clone());
                crate::interpreter::py_interop::py_class_mro(h)
                    .iter()
                    .any(|(m, n)| n == bare && module.as_deref().is_none_or(|want| want == m))
            }
            Value::CsObject(o) => {
                if o.class.name == type_name {
                    return true;
                }
                // 名前空間を通した型名（`cp.Person`）
                type_name.contains('.')
                    && self.dotted_class(type_name).is_some_and(|cls| {
                        std::rc::Rc::ptr_eq(&cls, &o.class) || cls.class_id == o.class.class_id
                    })
            }
            _ => false,
        }
    }

    /// 裸の名前 `name` を今の大域で引くと、`class` とは別のクラスか（known_bugs.md #30）。
    ///
    /// ⚠ 名前がクラスに束縛されていなければ `false`（名前での一致をそのまま使う）。テンプレート（`TemplateClass`・
    ///   `is Stack` はどの具体化にも真）・具体化の名前（`Stack[int]` は束縛ではない）・trait はここに来て、
    ///   以前と同じく名前で決まる。
    pub(crate) fn names_another_class(&self, class: &crate::interpreter::ClassValue, name: &str) -> bool {
        // 今の大域のモジュールで定義したクラスなら、引くまでもなくその名前はこのクラス（1 つのモジュールに同じ名前のクラスは
        // 1 つだけ・クラスの `module_name` は定義したときの `global_module_names`）。引くのは別のモジュールのクラスのときだけ。
        // ⚠ 名前を毎回引くと、`is` を回すだけのループが 15%（写さずに引いても 9%）遅くなった。
        // ⚠ 対話 REPL でクラスを定義し直すと、前のクラスのインスタンスも名前で一致する（以前と同じ）。
        // ⚠ 組み込みのクラス（`module_name` は本体と同じ `None`）は近道しない（本体の `class KeyError` と、モジュールの中で出た
        //   組み込みの `KeyError` を取り違える）。
        let here = self.global_module_names.get(self.cur_globals as usize).and_then(|n| n.as_deref());
        if class.module_name.as_deref() == here && !crate::interpreter::value::is_builtin_class_id(class.class_id) {
            return false;
        }
        // ⚠ 今の大域（`scopes[0]`）だけを引く。クラスとその `import` は最上位にしか書けない（10-16）ので、型の名前は大域の
        //   束縛。`get_var` はローカルの枠を先に見るが、VM で関数を呼んでも呼び出し側の枠は残るので、非同期タスクの本体が
        //   見ていた本体の `V`（タスクの枠に置かれる）をモジュールの関数の中から引いてしまっていた。
        // ⚠ 値を写さずに見る（`get_val` は値を写す）
        self.scopes[0].get(name).is_some_and(|var| {
            var.with_value(|v| match v {
                Value::Class(target) => !Self::same_class(class, target),
                // テンプレートの名前（`is Box`）は、そのテンプレートの具体化か（別のモジュールの同じ名前のテンプレートの
                // 具体化は通さない・known_bugs.md #39）
                Value::TemplateClass(t) => !self.is_instantiation_of(class, t),
                _ => false,
            })
        })
    }

    /// `class` がテンプレート `tmpl` の具体化か（known_bugs.md #39）: 具体化のクラス（`instance_name` を持つ）で、名前と
    /// 定義したモジュールが同じ。具体化は展開のときにテンプレートと同じモジュールの最上位で作る（`register_mono_instance`）。
    /// 1 つのモジュールに同じ名前のテンプレートは 1 つなので、別のモジュールの同じ名前のテンプレートとは区別できる。
    /// ⚠ テンプレートの番地（`template_class_cache` の鍵）では比べない。非同期タスクのワーカーでは名前空間の値と大域が
    ///   別々に写されるので、別名で引いたテンプレートと大域のテンプレートの番地が違う（実測）。
    pub(crate) fn is_instantiation_of(
        &self,
        class: &crate::interpreter::ClassValue,
        tmpl: &crate::interpreter::TemplateClassValue,
    ) -> bool {
        class.instance_name.is_some()
            && class.name == tmpl.name
            && class.module_name.as_deref() == tmpl.module_name.as_deref()
    }

    /// 今の大域（`scopes[0]`）で `name` に束縛されたクラスの `class_id`（クラスでなければ `None`）。
    /// コンパイル済みコードのガード（`ar_method_guard`）が使う。⚠ ローカルの枠は見ない（`names_another_class` と同じ理由）。
    pub(crate) fn global_class_id(&self, name: &str) -> Option<u32> {
        self.scopes[0].get(name).and_then(|var| {
            var.with_value(|v| match v {
                Value::Class(cls) => Some(cls.class_id),
                _ => None,
            })
        })
    }

    /// `class` が、名前で引いたクラス `target` と同じクラスか: `class_id` が同じか、`class` が enum のメンバーの
    /// クラスで `target` がその enum（`build_enum_classes` はメンバーごとに別のクラスを作るので `class_id` が違う。
    /// 同じモジュールの同じ名前の enum かで見る）。
    pub(crate) fn same_class(class: &crate::interpreter::ClassValue, target: &crate::interpreter::ClassValue) -> bool {
        target.class_id == class.class_id
            || class.enum_of.as_ref().is_some_and(|e| {
                target.enum_of.is_none() && &target.name == e && target.module_name == class.module_name
            })
    }

    /// `tags.Tag` / `tags.Box[int]` が、モジュール tags で定義したそのクラスか（10-8）。
    fn qualified_class_matches(class: &crate::interpreter::ClassValue, type_name: &str) -> bool {
        let Some(module) = class.module_name.as_deref() else {
            return false;
        };
        let Some(rest) = type_name.strip_prefix(module).and_then(|r| r.strip_prefix('.')) else {
            return false;
        };
        if rest == class.name {
            return true;
        }
        // 具体化の名前は空白を除いた正規形（`ClassValue::instance_name`）。
        match class.instance_name.as_deref() {
            Some(inst) => rest.chars().filter(|c| !c.is_whitespace()).eq(inst.chars()),
            None => false,
        }
    }

    /// `a.b`（名前空間を通した名前）が指す値。引けなければ `None`（7-27・Python のモジュールの名前空間を引く）。
    /// ⚠ 評価コア（`native` なし）でも使う（別名で書いたテンプレート `m.Box`・known_bugs.md #39）。
    fn dotted_value(&self, path: &str) -> Option<Value> {
        let mut parts = path.split('.');
        let mut cur = self.get_val(parts.next()?)?;
        for seg in parts {
            cur = match &cur {
                Value::Namespace(ns) => self.namespace_member(ns, seg)?,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// `t.Tag`（名前空間を通した型名）が指すクラス（10-8）。引けなければ `None`。
    fn dotted_class(&self, type_name: &str) -> Option<std::rc::Rc<crate::interpreter::ClassValue>> {
        match self.dotted_value(type_name)? {
            Value::Class(c) => Some(c),
            _ => None,
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
            Value::FrozenSet(_) => "frozenset".to_string(),
            Value::Bytes(_) => "bytes".to_string(),
            Value::Tuple(_) => "tuple".to_string(),
            Value::Function(f) => format!("function({})", f.name),
            Value::OverloadedFn(_) => "function(overloaded)".to_string(),
            Value::GeneratorFn(g) => format!("generator_fn({})", g.name),
            Value::NativeFunction(n) => format!("function({})", n.fn_name),
            Value::Ptr(p) => format!("Ptr[{}]", p.elem_type()),
            Value::Class(c) => format!("class({})", c.name),
            Value::Instance(i) => i.borrow().class.name.clone(),
            Value::CsObject(o) => o.class_name.clone(),
            Value::Slice(_) => "slice".to_string(),
            // Python のオブジェクトはそのクラスの名前（`mustbe` の誤りに出す・7-27）
            #[cfg(feature = "native")]
            Value::PyObject(h) => crate::interpreter::py_interop::py_class_mro(h)
                .first()
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "object".to_string()),
            _ => "unknown".to_string(),
        }
    }

}
