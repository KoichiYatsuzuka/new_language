// type_check/infer/attr.rs — 属性の読み出しの型と、メンバーの有無の判定。

use super::*;

/// **どのクラスのインスタンスにも生えているメソッド**（タスク 7.5）。
///
/// ⚠⚠ `interpreter/classes/method_call.rs` がクラスのメソッド表を引く**前に**
/// 特別扱いしているもの。実測で該当は `copy` だけ
/// （`keys` / `values` / `clear` / `items` / `get` / `add` / `append` / `pop` は
/// どれも `AttributeError: 'C' has no method ...`）。
/// ⚠ 実装側に共通メソッドを足したら**ここにも足すこと**。足し忘れると
/// 正しいコードが「存在しないメンバー」で落ちる。
const UNIVERSAL_INSTANCE_METHODS: &[&str] = &["copy"];

impl TypeChecker {
    /// 属性アクセス `obj.attr` の型を推論する。`Any`/`Union`/`Result` への
    /// アクセスは診断し、名前空間メンバーはその型を直接返す。
    pub(super) fn infer_attr(
        &mut self,
        object: &Expr,
        attr: &str,
        span: &Span,
        node_id: u32,
    ) -> InferredType {
        // ⚠ 入口で取り去る（受け手の `a.b` は読みとして検査する・`attr_is_callee` の doc）。
        let is_callee = std::mem::take(&mut self.attr_is_callee);
        let is_assign_target = std::mem::take(&mut self.attr_is_assign_target);
        let obj_ty = self.infer(object);
        // `import[cpp-dll] h as fp` の `fp.f`（今の Arrow から呼べない C の関数・1-2）。
        if let Expr::Ident { name, .. } = object {
            self.check_ffi_unsupported_use(&format!("{name}.{attr}"), name, Some(span.clone()));
        }
        // クラス・型の値の名前（`type(x).__name__` / `C.__name__`・python_builtins_plan.md のタスク 2-2）。
        // ⚠ dunder の名前だけ。`name` はクラス変数の名前とぶつかりうるので従来の経路に任せる。
        if matches!(obj_ty, InferredType::TypeValOf(_)) && matches!(attr, "__name__" | "__qualname__") {
            self.annotations.set_resolved(node_id, InferredType::Str);
            return InferredType::Str;
        }
        // 関数の値の名前と docstring（`f.__name__` / `f.__doc__`・python_builtins_plan.md のタスク 1-14）。
        // 実行時は `Interpreter::function_attr`。docstring が無ければ `None`。
        if matches!(obj_ty, InferredType::Function { .. }) {
            let ty = match attr {
                "__name__" | "__qualname__" => Some(InferredType::Str),
                "__doc__" => Some(InferredType::Union(vec![InferredType::Str, InferredType::None])),
                _ => None,
            };
            if let Some(ty) = ty {
                self.annotations.set_resolved(node_id, ty.clone());
                return ty;
            }
        }
        // ⚠⚠ **組み込みの値の属性**（フェーズ10 10-14）。以前は何も見ておらず、`x.name`（`x: int`）が
        //    実行時の `AttributeError` まで通っていた（テンプレートの具体化の本体も同じ）。
        //    - **読み**: 組み込みの値は読める属性を持たない。実行時の属性の読み（`get_attr_val`）は
        //      インスタンス・クラス・名前空間などにしか腕が無く、`int` も `str` も `list` も必ず
        //      `AttributeError` になる（メソッドは呼び出しの形でだけ使える）。
        //    - **呼び出し**: `int` / `float` / `bool` / `None` はメソッドを 1 つも持たない（実行時の
        //      メソッド呼び出しに腕が無い）。`complex` は `real` / `imag` / `angle` だけ。
        //      `str` / `list` などのメソッドの有無はここでは決めない（表を二重に持つとずれる）。
        if let Some(type_name) = Self::builtin_value_type(&obj_ty) {
            // 呼び出しの形は実行時に任せる（従来どおり）。**値として読む**形は、組み込みの値のメソッドの表
            // （`names::BUILTIN_VALUE_METHODS`・実行時が束縛メソッドを作る表と同じ）にある名前だけ（タスク 1-11）。
            let known_method = if let Some(kind) = Self::py_collection_kind(&obj_ty) {
                // `py_list` などは呼ぶ形も値として読む形も同じ表（中身は CPython の物・D-12・2-7）
                crate::type_check::names::is_py_collection_method(kind, attr)
            } else if is_callee {
                Self::builtin_may_have_method(&obj_ty, attr)
            } else {
                Self::builtin_method_kinds(&obj_ty)
                    .iter()
                    .any(|k| crate::type_check::names::is_builtin_value_method(k, attr))
            };
            if !known_method {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoSuchMember {
                        class_name: type_name,
                        member: attr.to_string(),
                    },
                    span: Some(span.clone()),
                });
            }
        }
        // ⚠⚠ **enum の型の値から引けるのはメンバーだけ**（タスク 2-1）。`Color.BLUE` はメンバーの表
        //    （`registry.enum_members`）を引く。`Color.value` のようなインスタンスのフィールドは
        //    引けない（実行時も `AttributeError: class 'Color' has no attribute 'value'`）。
        //    逆にインスタンス（`m: Color`）から引けるのは `value` だけで、下の一般の経路が
        //    `class_field_details` を引く（`m.BLUE` は `NoSuchMember`）。
        if let InferredType::TypeValOf(inner) = &obj_ty {
            if let InferredType::NamedInstance(enum_name) = inner.as_ref() {
                if let Some(members) = self.registry.enum_members(enum_name) {
                    let ty = match members.get(attr) {
                        Some(ty) => ty.clone(),
                        None => {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::NoSuchMember {
                                    class_name: enum_name.clone(),
                                    member: attr.to_string(),
                                },
                                span: Some(span.clone()),
                            });
                            InferredType::Unresolved
                        }
                    };
                    self.annotations.set_resolved(node_id, ty.clone());
                    return ty;
                }
            }
        }
        // ⚠ `GenericInstance{Box,[int]}` も**クラスとして扱う**（A-2）。あわせて
        //    型変数 → 具体型の置換表を取り出しておき、メンバーの型を置換してから返す。
        let class_subst = self.class_and_subst(&obj_ty).or_else(|| {
            // ⚠ **クラス名経由のアクセス**（`Color.Red` / `Counter.total` / `C.K`）も
            //    クラスのメンバー表を引く（タスク 2.3）。`Color` の型は
            //    `TypeValOf(NamedInstance("Color"))` なので `class_and_subst` では
            //    `None` になり、メンバーが全部 `Unresolved` になっていた。
            //    ⚠ enum のバリアント・`const` クラス変数・`static mut` が同じ経路に乗る。
            match &obj_ty {
                InferredType::TypeValOf(inner) => match inner.as_ref() {
                    InferredType::NamedInstance(c) => {
                        Some((c.clone(), std::collections::HashMap::new()))
                    }
                    // 具体化を値として使う形（`Counter[int].n`・10-9）は単相化した具体クラス。
                    InferredType::GenericInstance { .. } => self
                        .registry
                        .instance_class(&inner.to_string())
                        .map(|c| (c.clone(), std::collections::HashMap::new())),
                    _ => None,
                },
                _ => None,
            }
        });
        let class_name_opt = class_subst.as_ref().map(|(c, _)| c.clone());
        match &obj_ty {
            InferredType::Any => self.report_error(StaticTypeError {
                kind: TypeErrorKind::OperationOnAny {
                    op: "attribute access".to_string(),
                },
                span: Some(span.clone()),
            }),
            InferredType::Union(_) | InferredType::Result(_, _) => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::OperationOnUnion {
                        union_type: obj_ty.to_string(),
                        op: "attribute access".to_string(),
                    },
                    span: Some(span.clone()),
                });
            }
            // Intersection 型のメンバーアクセスはダウンキャストなしで許可する
            InferredType::Intersection(_) => {}
            _ => {}
        }
        // `__setattr__` を上書きした Python のクラスへの代入（`o.y = v`）は上書きが受ける（宣言していない名前も・
        // 値の型は上書きの仮引数が決める・known_bugs.md の #10）。
        if is_assign_target && class_name_opt.as_deref().is_some_and(|c| self.class_has_setattr_hook(c)) {
            self.annotations.set_resolved(node_id, InferredType::Any);
            return InferredType::Any;
        }
        if let Some(class_name) = &class_name_opt {
            self.check_member_access_static(class_name, attr, Some(span.clone()));
            // ⚠ メンバーの存在検査（タスク 7.5・検体 `M1` / `M2`）。
            // `__getattr__` を上書きした Python のクラスのインスタンスから読む無い名前は、上書きが答える（#11）。
            // ⚠ 代入の左辺は読みではない（`__setattr__` が無ければ宣言していない名前は書けない）。クラスから読む
            //   `C.zz` も対象外（CPython でもメタクラスの領分）。
            let from_instance =
                matches!(obj_ty, InferredType::NamedInstance(_) | InferredType::GenericInstance { .. });
            let hook = if is_assign_target || !from_instance { None } else { self.getattr_hook_type(class_name) };
            if let (false, Some(ty)) = (self.member_exists(class_name, attr), hook) {
                self.annotations.set_resolved(node_id, ty.clone());
                return ty;
            }
            if !self.member_exists(class_name, attr) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoSuchMember {
                        class_name: class_name.clone(),
                        member: attr.to_string(),
                    },
                    span: Some(span.clone()),
                });
            }
        }
        // ⚠ メンバーが確定している Arrow のモジュールに無い名前は静的エラー（2026-10-02）。
        //   実行時は `AttributeError`。メンバーには再エクスポート・サブモジュールも入っている
        //   （`TypeChecker::namespace_type`）。
        if let InferredType::Namespace(ref members, Some(ref closed), _) = obj_ty {
            if !members.contains_key(attr) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ModuleHasNoMember {
                        module: closed.name.clone(),
                        member: attr.to_string(),
                    },
                    span: Some(span.clone()),
                });
            }
        }
        // Namespace/PyNamespace はメンバ型、それ以外は解決不能。
        let fallback = if let InferredType::Namespace(ref members, _, _) = obj_ty {
            members.get(attr).cloned().unwrap_or(InferredType::Unresolved)
        } else if let InferredType::PyNamespace(ref members) = obj_ty {
            members.get(attr).cloned().unwrap_or(InferredType::Any)
        } else {
            InferredType::Unresolved
        };
        // ── AST 型解決層（#16）── 属性アクセスの型。`NamedInstance` のフィールドは registry から実型を引く。
        // **この実型をそのまま戻り値にする**（2026-08-10）。以前は注釈にだけ焼いて戻り値は `Unresolved` に
        // していたが、それだと `p.x * p.x` が `Unresolved` 同士の `BinOp` になり `binop_kind` が付かず、
        // VM の型特化 op もネイティブの型付き生成も効かなかった（#16 c-2 の結論「律速は型検査の解像度」）。
        // フィールドでない属性（メソッド名など）は registry に無いので従来どおり `fallback`。
        // ⚠⚠ **型引数で置換する**（A-2）。レジストリのフィールド型は置換前（`T`）なので、
        //    `Box[int]` から引いたまま返すと `T` が使用箇所へ漏れ、0-2/0-3 の照合が
        //    `declared 'T'` という偽陽性を出す（実測）。
        let field = class_subst.as_ref().and_then(|(class, map)| {
            // クラス側のディスクリプタは `__get__` の戻り値（インスタンスから読んでもクラスから読んでも・タスク 1-12）。
            if let Some(d) = self.class_descriptor(class, attr) {
                return Some(Self::subst_type_params(&d.read, map));
            }
            self.registry
                .class_field_details(class)
                .and_then(|m| m.get(attr))
                .map(|(_, ty)| Self::subst_type_params(ty, map))
        });
        // protocol の型の値のフィールド（`h.n`・`h: HasN[int]`）は、型引数で読んだ要求の型（6-5）。
        let field = field.or_else(|| match self.resolve_protocols(&obj_ty) {
            InferredType::Protocol(p, pargs) => {
                self.protocol_members(&p, &pargs).and_then(|(fields, _)| fields.get(attr).map(|(_, t)| t.clone()))
            }
            _ => None,
        });
        // メソッドを**値として**読む形（束縛メソッド・タスク 1-11）。フィールドが先（同名なら実行時もフィールド）。
        let bound = if field.is_none() && !is_callee {
            self.bound_method_type(object, &obj_ty, attr, class_subst.as_ref())
        } else {
            None
        };
        let resolved = field.or(bound).unwrap_or(fallback);
        self.annotations.set_resolved(node_id, resolved.clone());
        resolved
    }

    /// クラス側（`static mut` / `const`）に置いた**ディスクリプタ**（python_builtins_plan.md のタスク 1-12）。
    ///
    /// 宣言型が `__get__` を持つクラスのインスタンスなら、実行時は読むと `__get__(obj, C)` の戻り値、
    /// インスタンス経由で代入すると `__set__(obj, v)`（`eval/attrs.rs` のディスクリプタのプロトコル）。
    /// ⚠ インスタンスのフィールド（`mut f: D`）はディスクリプタとして働かない（CPython と同じ・実行時もクラス側だけを見る）。
    pub(crate) fn class_descriptor(&self, class_name: &str, attr: &str) -> Option<ClassDescriptor> {
        // 自クラスの表を先に引く（基底まで集めるのは見つからないときだけ・属性を読むたびに通るので）。
        let (kind, ty) = match self.registry.class_field_details(class_name).and_then(|m| m.get(attr)) {
            Some(entry) => entry.clone(),
            None => self.collect_class_field_details(class_name).remove(attr)?,
        };
        if !matches!(kind, crate::ast::FieldKind::Const | crate::ast::FieldKind::StaticMut) {
            return None;
        }
        let desc_class = match &ty {
            InferredType::NamedInstance(c) => c.clone(),
            InferredType::GenericInstance { .. } => self.registry.instance_class(&ty.to_string())?.clone(),
            _ => return None,
        };
        let sigs = self.collect_class_method_sigs(&desc_class);
        let get = sigs.get("__get__")?.first()?;
        let set = sigs.get("__set__").and_then(|s| s.first());
        Some(ClassDescriptor {
            read: get.return_type.clone().unwrap_or(InferredType::Unresolved),
            is_data: set.is_some() || sigs.contains_key("__delete__"),
            // `__set__(self, obj, value)` の `value`。
            write: set.map(|s| s.params.get(2).and_then(|(_, t)| t.clone()).unwrap_or(InferredType::Unresolved)),
            declared: ty,
        })
    }

    /// `py_list` / `py_dict` / `py_set` なら、メソッドの表（`names::PY_COLLECTION_METHODS`）を引く種類の名前（D-12・2-7）。
    pub(in crate::type_check) fn py_collection_kind(ty: &InferredType) -> Option<&'static str> {
        match ty {
            InferredType::PyList(_) => Some("py_list"),
            InferredType::PyDict(..) => Some("py_dict"),
            InferredType::PySet(_) => Some("py_set"),
            _ => None,
        }
    }

    /// 組み込みの値のメソッドの表（`names::BUILTIN_VALUE_METHODS`）を引く種類の名前。`list_like` は
    /// 実行時に list か fixed_list なので両方。メソッドを読めない型は空。
    pub(super) fn builtin_method_kinds(ty: &InferredType) -> &'static [&'static str] {
        use InferredType as T;
        match ty {
            T::Str => &["str"],
            T::Bytes => &["bytes"],
            T::List | T::ListOf(_) => &["list"],
            T::FixedList | T::FixedListOf(_) => &["fixed_list"],
            T::ListLike | T::ListLikeOf(_) => &["list", "fixed_list"],
            T::Dict | T::DictOf(_, _) => &["dict"],
            T::Set | T::SetOf(_) => &["set"],
            T::FrozenSet | T::FrozenSetOf(_) => &["frozenset"],
            T::IteratorOf(_) => &["generator"],
            _ => &[],
        }
    }

    /// `class_name` が `member` を持つか（タスク 7.5・検体 `M1` / `M2`）。
    ///
    /// ## ⚠⚠ Arrow のメンバーは**クラス本体の宣言だけで確定する**
    ///
    /// 実行時の文言がそう言っている:
    ///
    /// ```text
    /// AttributeError: 'Box' has no field 'newattr'; all fields must be declared in the class body
    /// ```
    ///
    /// `__init__` の中でも、他のメソッドでも、外からでも**宣言の無いフィールドは作れない**。
    /// ⇒ Arrow のクラスに関して存在検査は**完全に静的に決まる**。
    ///
    /// ## 判定の順序
    ///
    /// 1. **protocol** … 要求メンバー（`registry.protocol`）を見る
    /// 2. **フィールド** … 自クラス ＋ クラス継承 ＋ 基底 trait（`declared_field_type`）
    /// 3. **メソッド** … 継承チェーン込み ＋ 基底 trait のメソッド
    /// 4. **メンバー情報を 1 つも持たないクラス** … **開いている**とみなして素通し（下記）
    ///
    /// ## ⚠⚠ 「情報が無い」と「メンバーが無い」を取り違えないこと
    ///
    /// レジストリにフィールド表もメソッド表も基底も無いクラスは、
    /// **「メンバーが無い」のではなく「こちらが知らない」**。素通しする。実際に該当したもの:
    ///
    /// | 例 | 理由 |
    /// |---|---|
    /// | `slice` | 組み込みクラス。`begin`/`end`/`step` は `eval/attrs.rs` の特別扱い |
    /// | 関数の中で宣言した `enum` | 収集パスが拾っていない |
    /// | テンプレート型変数 `T` | `NamedInstance("T")` に化けているだけ |
    ///
    /// ⚠ これがタスク 7.8 で設計した `member_set_is_closed`（メンバー集合が閉じているか）の
    /// 実体。**スタブが入ったときに「スタブに載っているものだけ許す」へ寄せられる形**に
    /// してある。真偽値ではなく**クラス単位の性質**として持つこと。
    /// `__setattr__` を上書きしたクラスか（known_bugs.md の #10）。持てるのは Python のクラスだけ（Arrow のクラスでは
    /// パーサの誤り）。基底のものも含む（変換したクラスのメソッドは祖先まで平坦化してある）。
    pub(in crate::type_check) fn class_has_setattr_hook(&self, class_name: &str) -> bool {
        self.collect_class_method_sigs(class_name).contains_key("__setattr__")
    }

    /// 読みを上書きしたクラス（`__getattribute__`・#12 / `__getattr__`・#11）なら、その戻り値の型（注釈が無ければ `Any`・
    /// 両方あれば `__getattribute__`）。宣言していない名前をインスタンスから読む・呼ぶ形はこれが答える（known_bugs.md）。
    /// 持てるのは Python のクラスだけ（`class_has_setattr_hook` と同じ）。
    pub(in crate::type_check) fn getattr_hook_type(&self, class_name: &str) -> Option<InferredType> {
        let sigs = self.collect_class_method_sigs(class_name);
        let hook = sigs.get("__getattribute__").or_else(|| sigs.get("__getattr__"))?;
        let ret = hook.first().and_then(|s| s.return_type.clone());
        Some(match ret {
            Some(t) if !matches!(t, InferredType::Unresolved) => t,
            _ => InferredType::Any,
        })
    }

    pub(in crate::type_check) fn member_exists(&self, class_name: &str, member: &str) -> bool {
        // 0. ⚠⚠ **全インスタンス共通のメソッド**。`method_call.rs` がクラスのメソッド表を
        //    引く**前に**特別扱いしているので、どのクラスにも生えている。
        //    実測で確かめた（`copy` だけが該当。`keys` / `values` / `clear` / `items` /
        //    `get` / `add` / `append` / `pop` はどれも `AttributeError`）。
        // ⚠ 実装側に共通メソッドを足したら**ここにも足すこと**。
        if UNIVERSAL_INSTANCE_METHODS.contains(&member) {
            return true;
        }
        // ⚠⚠ 未展開の `!装飾子` を持つクラスは**メンバーの顔ぶれが未確定**（タスク 2-4）。
        //    装飾子が何を足すか分からないので「無い」と言い切れない。⇒ 通す。
        if self.registry.members_unresolved(class_name) {
            return true;
        }
        // 1. protocol は要求メンバーが別の表にある。
        if let Some(p) = self.registry.protocol(class_name) {
            return p.fields.iter().any(|f| f.name == member)
                || p.methods.iter().any(|m| m.name == member);
        }
        // 2/3. フィールドとメソッド。
        if self.declared_field_type_pub(class_name, member).is_some() {
            return true;
        }
        if self.collect_class_method_sigs(class_name).contains_key(member) {
            return true;
        }
        for base in self.registry.class_bases(class_name).unwrap_or(&[]) {
            if self
                .registry
                .trait_methods(base)
                .is_some_and(|m| m.contains_key(member))
            {
                return true;
            }
            if self
                .registry
                .trait_field_details(base)
                .is_some_and(|m| m.contains_key(member))
            {
                return true;
            }
        }
        // 4. メンバー集合が閉じていない（＝こちらが何も知らない）クラスは素通し。
        !self.member_set_is_closed(class_name)
    }

    /// 属性を読めない組み込みの値の型なら、その表示名（`int` / `list[int]` …・10-14）。
    ///
    /// ⚠ `generator`（`NamedInstance`）・クラス・名前空間・`Any` / `Unresolved` などは対象外。
    fn builtin_value_type(ty: &InferredType) -> Option<String> {
        use InferredType as T;
        match ty {
            T::Int
            | T::Float
            | T::Complex
            | T::Bool
            | T::Str
            | T::None
            | T::List
            | T::ListOf(_)
            | T::FixedList
            | T::FixedListOf(_)
            | T::ListLike
            | T::ListLikeOf(_)
            | T::Dict
            | T::DictOf(_, _)
            | T::Set
            | T::SetOf(_)
            | T::FrozenSet
            | T::FrozenSetOf(_)
            | T::Tuple(_)
            | T::TupleAny
            | T::IteratorOf(_)
            | T::PyList(_)
            | T::PyDict(..)
            | T::PySet(_) => Some(ty.to_string()),
            _ => None,
        }
    }

    /// 組み込みの値のメソッドとして**ありうるか**（10-14）。`false` は「実行時に必ず無い」。
    fn builtin_may_have_method(ty: &InferredType, method: &str) -> bool {
        use InferredType as T;
        // 組み込みの値の表のメンバー（`(5).__index__()` など・6-6）は呼べる（実行時は `call_builtin_member`）
        if crate::type_check::builtin_members::shape_of(ty)
            .is_some_and(|s| crate::type_check::builtin_members::has_member(s.kind, method))
        {
            return true;
        }
        match ty {
            // 実行時のメソッド呼び出し（`eval_method_call_full`）に腕が無い。
            T::Int | T::Float | T::Bool | T::None => false,
            T::Complex => matches!(method, "real" | "imag" | "angle"),
            // `str` / 容器のメソッドの表は実行時にしか無いので、ここでは決めない。
            _ => true,
        }
    }

    /// クラスの**メンバー集合が閉じている**か（タスク 7.5 / 7.8）。
    ///
    /// ⚠⚠ **真偽値に潰さずクラス単位の性質として持つこと。** 外部言語のスタブが入ったとき、
    /// 言語によって「スタブ＝完全な宣言（閉じる）」「スタブ＝部分的な宣言（開いたまま）」が
    /// 分かれる。ここを「外部由来なら飛ばす」にすると**その区別が表現できなくなる**。
    ///
    /// 現状の規則: **レジストリに何らかのメンバー情報があれば閉じている**。
    /// Arrow のクラス宣言（`.ar` / `.ars` スタブ由来を含む）は必ず表を持つので閉じる。
    fn member_set_is_closed(&self, class_name: &str) -> bool {
        self.registry.class_field_details(class_name).is_some()
            || self.registry.class_methods(class_name).is_some()
            || self
                .registry
                .class_bases(class_name)
                .is_some_and(|b| !b.is_empty())
    }
}
