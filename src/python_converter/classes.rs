// python_converter/classes.rs — クラス・パラメータの変換: convert_class / self フィールド収集 / パラメータ型抽出 / convert_params。

use {
    rustpython_parser::ast as py,
    crate::ast::{FieldKind, Param, Stmt, PY_KWARGS_PARAM},
};
use super::*;

// ---------------------------------------------------------------------------
// クラス変換
// ---------------------------------------------------------------------------

/// Python クラス定義を tl の `Stmt::ClassDef` に変換する。
/// フィールドは `__init__` からの `self.x = ...` 代入と型アノテーションを元に収集する。
///
/// 戻り値は**文の並び**: クラス本体の中のクラス（入れ子のクラス）を先に、このクラスを最後に置く（フェーズ10 10-18）。
/// - 入れ子のクラスは**このクラスと同じ場所へ持ち上げ**、名前を Python の __qualname__（`Outer.Inner`）にする。
///   ⚠ 素の名前（`Inner`）で持ち上げると、最上位の同名のクラスや別のクラスの同名の入れ子（`A.Meta` と
///   `B.Meta`）とぶつかる。
/// - このクラスにはクラス変数 `Inner = Outer.Inner` を置く（`Outer.Inner()` / `self.Inner()` が引ける）。
/// ⚠ 以前はクラス本体の `class` を**黙って捨てていた**（`class 'Outer' has no method 'Inner'`）。
pub(crate) fn convert_class(c: &py::StmtClassDef, filename: &str) -> Result<Vec<Stmt>, String> {
    convert_class_in(c, filename, c.name.to_string(), &std::collections::HashMap::new())
}

/// [`convert_class`] の本体。`qualname` はこのクラスの名前（入れ子なら `Outer.Inner`）、
/// `siblings` は同じクラス本体で先に定義された入れ子のクラスの素の名前 → 持ち上げた名前。
///
/// ⚠ Python のクラス本体の中では、先に定義した入れ子のクラスを素の名前で引ける（`class B(A):` の `A`）。
///   持ち上げると最上位の名前で引くことになるので、基底の名前はここで持ち上げた名前へ置き換える。
fn convert_class_in(
    c: &py::StmtClassDef,
    filename: &str,
    qualname: String,
    siblings: &std::collections::HashMap<String, String>,
) -> Result<Vec<Stmt>, String> {
    let class_name = qualname;
    let bases: Vec<String> = c
        .bases
        .iter()
        .map(expr_to_name)
        .map(|b| siblings.get(&b).cloned().unwrap_or(b))
        .collect();
    // `super()` の脱糖用に**第 1 基底**を積む（メソッド本体の変換中だけ有効）。
    // ⚠ 多重継承では 1 番目だけを見る（Python の MRO とは違うが、単一継承では一致する）。
    let _super_guard = SuperBaseGuard::push(bases.first().cloned());
    // クラスデコレータ。`@staticmethod` 等はクラスには付かないので `in_class: false`。
    let class_dec = convert_decorators(
        &c.decorator_list,
        filename,
        &format!("class '{class_name}'"),
        false,
    )?;

    let mut fields: Vec<Stmt> = Vec::new();
    let mut methods: Vec<Stmt> = Vec::new();
    // 持ち上げた入れ子のクラス（このクラスより前に置く）と、その素の名前 → 持ち上げた名前（10-18）。
    let mut hoisted: Vec<Stmt> = Vec::new();
    let mut nested: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // property を組み立てる文（クラスの定義の後に置く・タスク 6-1）と、その状態。
    let mut after_class: Vec<Stmt> = Vec::new();
    let mut defined_props: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prop_method_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // 変換しない文を明示エラーにする（黙って捨てない・10-18）。
    let unsupported = |what: &str| -> Result<(), String> {
        Err(format!(
            "{filename}: {what} in the body of class '{class_name}' is not supported \
             (only methods, class attributes and nested classes are converted)"
        ))
    };

    // property の名前（`@property` / `@x.setter` などをどこかに持つメソッド・タスク 6-1 / 6-6）。
    // ⚠ `self.x = v` は setter を呼ぶ代入なので、インスタンスのフィールドにしない。
    let prop_names: std::collections::HashSet<String> = c
        .body
        .iter()
        .filter_map(|s| match s {
            py::Stmt::FunctionDef(f) if f.decorator_list.iter().any(|d| is_property_decorator(Some(d))) => {
                Some(f.name.to_string())
            }
            _ => None,
        })
        .collect();

    // __init__ を先に見つけてインスタンスフィールドを収集
    let mut init_fields: Vec<(String, String)> = Vec::new();
    for stmt in &c.body {
        if let py::Stmt::FunctionDef(f) = stmt {
            if f.name.as_str() == "__init__" {
                collect_self_fields(&f.body, &mut init_fields);
                let param_types = extract_param_types(&f.args);
                for (fname, ftype) in init_fields.iter_mut() {
                    if ftype == "Any" {
                        if let Some(t) = param_types.get(fname.as_str()) {
                            *ftype = t.clone();
                        }
                    }
                }
                break;
            }
        }
    }
    // ほかのメソッドだけが代入するフィールド（setter の `self._c = v` など・タスク 6-1）。`__init__` の後ろに並べる。
    // ⚠ Python では どのメソッドでもインスタンスの属性を作れる。以前は `__init__` だけを見ていたので、
    //   `__init__` が setter を通して値を入れる形（`self.celsius = c` → setter が `self._c = v`）が
    //   `has no field '_c'` で動かなかった。
    // ⚠ クラス属性・メソッドと同じ名前は足さない（クラス属性は従来どおり `static mut` を書き換える）。
    {
        let class_level: std::collections::HashSet<&str> = c
            .body
            .iter()
            .filter_map(|s| match s {
                py::Stmt::FunctionDef(f) => Some(f.name.as_str()),
                py::Stmt::Assign(a) => match a.targets.as_slice() {
                    [py::Expr::Name(n)] => Some(n.id.as_str()),
                    _ => None,
                },
                py::Stmt::AnnAssign(a) => match &*a.target {
                    py::Expr::Name(n) => Some(n.id.as_str()),
                    _ => None,
                },
                py::Stmt::ClassDef(k) => Some(k.name.as_str()),
                _ => None,
            })
            .collect();
        let mut more: Vec<(String, String)> = Vec::new();
        for stmt in &c.body {
            if let py::Stmt::FunctionDef(f) = stmt {
                if f.name.as_str() != "__init__" {
                    collect_self_fields(&f.body, &mut more);
                }
            }
        }
        for (fname, ftype) in more {
            if !class_level.contains(fname.as_str()) && !init_fields.iter().any(|(n, _)| *n == fname) {
                init_fields.push((fname, ftype));
            }
        }
    }
    init_fields.retain(|(n, _)| !prop_names.contains(n));

    let mut seen_fields: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (fname, ftype) in &init_fields {
        if seen_fields.insert(fname.clone()) {
            fields.push(Stmt::Field { src: None,
                name: fname.clone(),
                kind: FieldKind::Mut,
                type_ann: ftype.clone(),
                default: None,
                access: crate::ast::Accessibility::Public,
            });
        }
    }

    for stmt in &c.body {
        match stmt {
            py::Stmt::Assign(a) if a.targets.len() == 1 => {
                if let py::Expr::Name(n) = &a.targets[0] {
                    let fname = n.id.to_string();
                    if !fname.starts_with("__") {
                        let default = convert_expr(&a.value, filename)?;
                        if seen_fields.insert(fname.clone()) {
                            fields.push(Stmt::Field { src: None,
                                name: fname,
                                // ⚠ Python のクラス属性は**可変・全インスタンス共有**なので
                                //   `StaticMut`（`static mut`）に対応する。`Const` にすると
                                //   `Counter.count = ...` が
                                //   `cannot assign to class variable (declared const)` で落ちる。
                                //   定数として使いたい属性と静的に区別できないため、
                                //   Python 側の意味に忠実な**可変**へ倒す。
                                kind: FieldKind::StaticMut,
                                type_ann: "Any".to_string(),
                                default: Some(default),
                                access: crate::ast::Accessibility::Public,
                            });
                        }
                    }
                } else {
                    unsupported("an assignment to something other than a single name")?
                }
            }
            py::Stmt::AnnAssign(a) => {
                if let py::Expr::Name(n) = &*a.target {
                    let fname = n.id.to_string();
                    if !fname.starts_with("__") {
                        let type_ann = convert_annotation(&a.annotation);
                        if let Some(val_expr) = &a.value {
                            let default = convert_expr(val_expr, filename)?;
                            if seen_fields.insert(fname.clone()) {
                                fields.push(Stmt::Field { src: None,
                                    name: fname,
                                    // 注釈つきクラス変数 `n: int = 5` も同じく共有可変。
                                    kind: FieldKind::StaticMut,
                                    type_ann,
                                    default: Some(default),
                                    access: crate::ast::Accessibility::Public,
                                });
                            }
                        }
                    }
                } else {
                    unsupported("an annotated assignment to something other than a name")?
                }
            }
            py::Stmt::FunctionDef(f) => {
                // `@property` / `@x.setter` は取り除いて別に扱う（タスク 6-1）。それより内側のデコレータは
                // メソッドに、外側のデコレータは組み立てた property に当てる（タスク 6-6）。
                let found = property_decorator(f, &class_name, filename)?;
                let (prop, rest, outer) = match found {
                    None => (None, &f.decorator_list[..], Vec::new()),
                    Some((p, pos)) => {
                        let mut outer = Vec::with_capacity(pos);
                        for d in &f.decorator_list[..pos] {
                            let name = expr_to_name(match d {
                                py::Expr::Call(c) => &c.func,
                                other => other,
                            });
                            let last = name.rsplit('.').next().unwrap_or("");
                            if matches!(last, "staticmethod" | "classmethod" | "abstractmethod" | "cached_property") {
                                return Err(format!(
                                    "{filename}: '@{name}' outside a property decorator is not supported \
                                     (method '{}.{}')",
                                    class_name,
                                    f.name.as_str()
                                ));
                            }
                            outer.push(convert_expr(d, filename)?);
                        }
                        (Some(p), &f.decorator_list[pos + 1..], outer)
                    }
                };
                let dec = convert_decorators(
                    rest,
                    filename,
                    &format!("method '{}.{}'", class_name, f.name.as_str()),
                    true,
                )?;
                // property のメソッドは別名にする（`x` はクラスの属性＝ property の置き場）。
                let method_name = match &prop {
                    None => f.name.to_string(),
                    Some(p) => {
                        if dec.is_static || dec.is_class_method {
                            return Err(format!(
                                "{filename}: a property cannot be a static or class method (method '{}.{}')",
                                class_name,
                                f.name.as_str()
                            ));
                        }
                        let base = format!("__property_{}_{}", p.kind().tag(), f.name.as_str());
                        let n = prop_method_count.entry(base.clone()).or_insert(0usize);
                        *n += 1;
                        if *n == 1 { base } else { format!("{base}_{n}") }
                    }
                };
                if let Some(p) = prop {
                    let pname = f.name.to_string();
                    // クラスの属性の置き場（クラスの定義の後で property を入れる）。
                    if seen_fields.insert(pname.clone()) {
                        fields.push(Stmt::Field { src: None,
                            name: pname.clone(),
                            kind: FieldKind::StaticMut,
                            type_ann: "Any".to_string(),
                            default: Some(crate::ast::Expr::None),
                            access: crate::ast::Accessibility::Public,
                        });
                    }
                    // メソッドの docstring（本体の最初の文字列の式・タスク 6-7）。
                    let doc = match f.body.first() {
                        Some(py::Stmt::Expr(e)) => match &*e.value {
                            py::Expr::Constant(k) => match &k.value {
                                py::Constant::Str(s) => Some(s.to_string()),
                                _ => None,
                            },
                            _ => None,
                        },
                        _ => None,
                    };
                    after_class.extend(property_stmts(
                        &class_name,
                        &pname,
                        &method_name,
                        p,
                        outer,
                        doc,
                        &mut defined_props,
                        filename,
                    )?);
                }
                let (params, renames) = convert_params(&f.args, filename)?;
                let return_type = f.returns.as_deref().map(convert_annotation);
                // メソッド本体は**新しいスコープ**。パラメータ名（`self` 含む）を宣言済みとして渡す。
                let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
                let body = {
                    // `*args` / `**kwargs` の識別子差し替えは**この本体の変換中だけ**有効。
                    let _rename_guard = ParamRenameGuard::push(renames, &param_names);
                    let _body_guard = crate::python_converter::statements::FnBodyGuard::enter();
                    convert_scope(&f.body, filename, &param_names)?
                };
                // ★ 本体に文としての `yield` があれば `gen` メソッド（項目 9）。
                //   ⚠ `Stmt::GenDef` は `decorators` / `is_static` / `is_class_method` /
                //     `is_abstract` を持たない（Arrow の `gen` にこれらの形が無い）。
                //     黙って捨てると効かなくなるので、付いていれば明示エラーにする。
                if frame_has_yield(&f.body) {
                    let bad = if !dec.decorators.is_empty() {
                        Some("a decorator")
                    } else if dec.is_static {
                        Some("`@staticmethod`")
                    } else if dec.is_class_method {
                        Some("`@classmethod`")
                    } else if dec.is_abstract {
                        Some("`@abstractmethod`")
                    } else {
                        None
                    };
                    if let Some(what) = bad {
                        return Err(format!(
                            "{filename}: {what} on a generator method is not supported \
                             (method '{}.{}'); Arrow's `gen` has no such form",
                            class_name,
                            f.name.as_str()
                        ));
                    }
                    methods.push(Stmt::GenDef { src: None,
                        name: method_name,
                        template_params: vec![],
                        params,
                        yield_type: extract_yield_type(f.returns.as_deref()),
                        body,
                        access: crate::ast::Accessibility::Public,
                    });
                    continue;
                }
                methods.push(Stmt::FnDef { src: None,
                    name: method_name,
                    template_params: vec![],
                    params,
                    return_type,
                    body,
                    is_abstract: dec.is_abstract,
                    is_static: dec.is_static,
                    is_class_method: dec.is_class_method,
                    decorators: dec.decorators,
                    access: crate::ast::Accessibility::Public,
                });
            }
            // 入れ子のクラス（10-18）: 持ち上げて、このクラスにはクラス変数で結ぶ。
            py::Stmt::ClassDef(inner) => {
                let inner_name = inner.name.to_string();
                let inner_q = format!("{class_name}.{inner_name}");
                hoisted.extend(convert_class_in(inner, filename, inner_q.clone(), &nested)?);
                nested.insert(inner_name.clone(), inner_q.clone());
                if seen_fields.insert(inner_name.clone()) {
                    fields.push(Stmt::Field { src: None,
                        name: inner_name,
                        // Python のクラス属性と同じく共有可変（上の `Assign` の腕と同じ理由）。
                        kind: FieldKind::StaticMut,
                        // 型はそのクラス自身（`type[Outer.Inner]`）。`Any` にすると Arrow から
                        // `m.Deep.Mid.Leaf()` と辿れない（`Any` の属性は静的エラー）。
                        type_ann: format!("type[{inner_q}]"),
                        default: Some(crate::ast::Expr::Ident {
                            name: inner_q,
                            node_id: 0,
                            res: crate::ast::Resolution::Unresolved,
                        }),
                        access: crate::ast::Accessibility::Public,
                    });
                }
            }
            py::Stmt::Pass(_) => {}
            // docstring と `...`（空の本体）は実行しても何も起きない。
            py::Stmt::Expr(e)
                if matches!(
                    &*e.value,
                    py::Expr::Constant(k)
                        if matches!(k.value, py::Constant::Str(_) | py::Constant::Ellipsis)
                ) => {}
            // ⚠ 上の腕が受け取らなかった形は**黙って捨てない**（10-18）。以前はクラス本体の `class` /
            //   `if` / `import` / 複数の代入先などを捨てていて、実行時に「無い」と言われるまで分からなかった。
            py::Stmt::Assign(_) => unsupported("an assignment to something other than a single name")?,
            py::Stmt::AsyncFunctionDef(f) => {
                unsupported(&format!("an async method (`async def {}`)", f.name.as_str()))?
            }
            py::Stmt::Import(_) | py::Stmt::ImportFrom(_) => unsupported("an `import`")?,
            other => unsupported(py_stmt_kind(other))?,
        }
    }

    let mut body = fields;
    body.extend(methods);

    hoisted.push(Stmt::ClassDef { src: None,
        name: class_name,
        template_params: vec![],
        // ⚠ Python 由来のクラスに trait の型引数は無い（タスク 9.9）。
        base_args: vec![Vec::new(); bases.len()],
        bases,
        body,
        decorators: class_dec.decorators,
    });
    hoisted.extend(after_class);
    Ok(hoisted)
}

// ---------------------------------------------------------------------------
// property（python_builtins_plan.md のタスク 6-1）
// ---------------------------------------------------------------------------
//
// CPython の `@property def x` はクラス本体の名前空間に property を作り、`@x.setter def x` はそれを写して
// setter を足したものに置き換える。Arrow のクラス本体は名前空間ではない（クラス属性の初期化式からクラス本体の
// 名前も、作っている最中のクラスも見えない）ので、**クラスを定義した後で同じものを組み立てる**:
//
//   class C:                                   class C:
//       @property                                  static mut x: Any = None
//       def x(self): ..                ⇒           fn __property_get_x(self): ..
//       @x.setter                                  fn __property_set_x(self, v): ..
//       def x(self, v): ..                     C.x = property(C.__property_get_x)
//                                              C.x.__set_name__(C, "x")
//                                              C.x = C.x.setter(C.__property_set_x)
//
// 読み書きは組み込みの `property`（`eval/py_builtins.rs`）とディスクリプタのプロトコル（タスク 1-12）が受ける。
// ⚠ `__set_name__` は CPython ではクラスを作るときに `type.__new__` が呼ぶ（誤りの文言に名前を入れるため）。
// ⚠ `x = property(getx, setx)`（クラス属性に直接書く形）はクラス本体の名前を参照するので、まだ動かない。

/// property のデコレータの種類（`@x.getter` / `@x.setter` / `@x.deleter`。`@property` は getter）。
#[derive(Clone, Copy)]
enum PropKind {
    Get,
    Set,
    Del,
}

impl PropKind {
    /// 別名のメソッドに入れる印（`__property_get_x`）。
    fn tag(self) -> &'static str {
        match self {
            PropKind::Get => "get",
            PropKind::Set => "set",
            PropKind::Del => "del",
        }
    }

    /// property のメソッドの名前（`C.x.setter(..)`）。
    fn method(self) -> &'static str {
        match self {
            PropKind::Get => "getter",
            PropKind::Set => "setter",
            PropKind::Del => "deleter",
        }
    }
}

/// メソッドのいちばん外側の property のデコレータ。
enum PropDecor {
    /// `@property`
    New,
    /// `@x.setter` など（`owner` は `@Base.x.setter` の `Base`。このクラスの `x` なら `None`）。
    Accessor { owner: Option<py::Expr>, kind: PropKind },
}

impl PropDecor {
    fn kind(&self) -> PropKind {
        match self {
            PropDecor::New => PropKind::Get,
            PropDecor::Accessor { kind, .. } => *kind,
        }
    }
}

/// デコレータが property のもの（`@property` / `@x.setter` / `@Base.x.getter` …）か。
fn is_property_decorator(dec: Option<&py::Expr>) -> bool {
    match dec {
        Some(py::Expr::Name(n)) => n.id.as_str() == "property",
        Some(py::Expr::Attribute(a)) => {
            matches!(a.attr.as_str(), "getter" | "setter" | "deleter")
                && matches!(&*a.value, py::Expr::Name(_) | py::Expr::Attribute(_))
        }
        _ => false,
    }
}

/// `f` のデコレータの並びに property のものがあれば、その形と並びの中の位置。
///
/// 位置より**外側**（前）のデコレータは組み立てた property を包み（`@logged @property` は CPython の
/// `logged(property(f))`・タスク 6-6）、**内側**（後ろ）のデコレータはメソッドそのものに当たる
/// （`@property @abstractmethod`）。⚠ property のデコレータが 2 つあるのは誤り。
fn property_decorator(
    f: &py::StmtFunctionDef,
    class_name: &str,
    filename: &str,
) -> Result<Option<(PropDecor, usize)>, String> {
    let mut found = f.decorator_list.iter().enumerate().filter(|(_, d)| is_property_decorator(Some(d)));
    let Some((pos, dec)) = found.next() else {
        return Ok(None);
    };
    if found.next().is_some() {
        return Err(format!(
            "{filename}: method '{class_name}.{}' has more than one property decorator",
            f.name.as_str()
        ));
    }
    let py::Expr::Attribute(a) = dec else {
        return Ok(Some((PropDecor::New, pos)));
    };
    let kind = match a.attr.as_str() {
        "getter" => PropKind::Get,
        "setter" => PropKind::Set,
        _ => PropKind::Del,
    };
    let (owner, prop) = match &*a.value {
        py::Expr::Attribute(inner) => (Some((*inner.value).clone()), inner.attr.to_string()),
        other => (None, expr_to_name(other)),
    };
    // `@x.setter def set_x(..)` は CPython では `x` を写した別の property `set_x` を作る（`x` は getter だけのまま）。
    // 意図と違うことがほとんどなので、表さずに誤りにする。
    if prop != f.name.as_str() {
        return Err(format!(
            "{filename}: '@{}' on method '{class_name}.{}' is not supported: \
             the method must have the property's name '{prop}'",
            expr_to_name(&py::Expr::Attribute(a.clone())),
            f.name.as_str()
        ));
    }
    Ok(Some((PropDecor::Accessor { owner, kind }, pos)))
}

/// property を組み立てる文（クラスの定義の後に置く）。`method` は別名にしたメソッドの名前。
/// `outer` は property より外側のデコレータ（変換済みの式・外から順）。組み立てて名前を付けた後で
/// `C.x = outer[0](outer[1](C.x))` と包む（タスク 6-6）。
/// ⚠ 名前（`__set_name__`）は包む前の property に付ける。CPython は包んだ後の値に付けるので、包みが
///   property と別の物を返すときだけ違う（その property の誤りの文言に名前が入るかどうか）。
///
/// `doc` はメソッドの docstring（タスク 6-7）。CPython と同じく getter の docstring が property の `__doc__` になる:
/// `@property` は `property(f, None, None, doc)`、`@x.getter` は差し替えた後で `C.x.__doc__ = doc`（docstring が無ければ
/// `None`。CPython も前の getter の doc を捨てる）。setter / deleter の docstring は使わない。
#[allow(clippy::too_many_arguments)]
fn property_stmts(
    class_name: &str,
    prop: &str,
    method: &str,
    decor: PropDecor,
    outer: Vec<crate::ast::Expr>,
    doc: Option<String>,
    defined: &mut std::collections::HashSet<String>,
    filename: &str,
) -> Result<Vec<Stmt>, String> {
    use crate::ast::{CallArg, Expr, Resolution};
    let ident = |name: &str| Expr::Ident { name: name.to_string(), node_id: 0, res: Resolution::Unresolved };
    let attr = |object: Expr, name: &str| Expr::Attr {
        object: Box::new(object),
        attr: name.to_string(),
        span: make_span(filename),
        cache: Default::default(),
        node_id: 0,
    };
    let call = |func: Expr, args: Vec<Expr>| Expr::Call {
        func: Box::new(func),
        args: args.into_iter().map(CallArg::Positional).collect(),
        span: crate::token::Span::unknown(),
        cache: Default::default(),
        node_id: 0,
    };
    let func = attr(ident(class_name), method);
    let kind = decor.kind();
    let doc_expr = || match &doc {
        Some(d) => Expr::Str(std::rc::Rc::from(d.as_str())),
        None => Expr::None,
    };
    // `@x.getter` / `@Base.x.getter` は差し替えた後で doc を付け直す（`@property` は構築子に渡す）。
    let regetter = matches!(decor, PropDecor::Accessor { .. }) && matches!(kind, PropKind::Get);
    let (value, name_it) = match decor {
        PropDecor::New => match &doc {
            Some(_) => (call(ident("property"), vec![func, Expr::None, Expr::None, doc_expr()]), true),
            None => (call(ident("property"), vec![func]), true),
        },
        PropDecor::Accessor { owner: None, .. } => {
            // CPython ではクラス本体の名前 `x` を引く。このクラスで先に `@property def x` が無ければ引けない。
            if !defined.contains(prop) {
                return Err(format!(
                    "{filename}: '@{prop}.{}' in class '{class_name}' must follow '@property def {prop}' \
                     in the same class (use '@Base.{prop}.{}' to extend a base class's property)",
                    kind.method(),
                    kind.method()
                ));
            }
            (call(attr(attr(ident(class_name), prop), kind.method()), vec![func]), false)
        }
        // 基底の property を写す（`@Base.x.getter`）。名前はこのクラスで付け直す（CPython も `type.__new__` が付ける）。
        PropDecor::Accessor { owner: Some(owner), .. } => {
            let owner = convert_expr(&owner, filename)?;
            (call(attr(attr(owner, prop), kind.method()), vec![func]), true)
        }
    };
    defined.insert(prop.to_string());
    let mut out = vec![Stmt::AttrAssign {
        target: attr(ident(class_name), prop),
        value,
        span: crate::token::Span::unknown(),
    }];
    if name_it {
        out.push(Stmt::Expr(call(
            attr(attr(ident(class_name), prop), "__set_name__"),
            vec![ident(class_name), Expr::Str(std::rc::Rc::from(prop))],
        )));
    }
    if regetter {
        out.push(Stmt::AttrAssign {
            target: attr(attr(ident(class_name), prop), "__doc__"),
            value: doc_expr(),
            span: crate::token::Span::unknown(),
        });
    }
    if !outer.is_empty() {
        let wrapped = outer
            .into_iter()
            .rev()
            .fold(attr(ident(class_name), prop), |inner, deco| call(deco, vec![inner]));
        out.push(Stmt::AttrAssign {
            target: attr(ident(class_name), prop),
            value: wrapped,
            span: crate::token::Span::unknown(),
        });
    }
    Ok(out)
}

/// 変換しない Python の文の種類（誤りの文面用・冠詞つき・10-18）。
fn py_stmt_kind(s: &py::Stmt) -> &'static str {
    match s {
        py::Stmt::If(_) => "an `if` statement",
        py::Stmt::For(_) | py::Stmt::AsyncFor(_) => "a `for` statement",
        py::Stmt::While(_) => "a `while` statement",
        py::Stmt::Try(_) | py::Stmt::TryStar(_) => "a `try` statement",
        py::Stmt::With(_) | py::Stmt::AsyncWith(_) => "a `with` statement",
        py::Stmt::Match(_) => "a `match` statement",
        py::Stmt::AugAssign(_) => "an augmented assignment (`+=` etc.)",
        py::Stmt::Delete(_) => "a `del` statement",
        py::Stmt::Expr(_) => "an expression statement",
        py::Stmt::Raise(_) => "a `raise` statement",
        py::Stmt::Assert(_) => "an `assert` statement",
        py::Stmt::Global(_) => "a `global` statement",
        py::Stmt::Nonlocal(_) => "a `nonlocal` statement",
        py::Stmt::Return(_) => "a `return` statement",
        _ => "a statement of this kind",
    }
}

/// `__init__` 本体（ネスト含む）を再帰探索して `self.field = ...` の代入を収集する。
pub(crate) fn collect_self_fields(stmts: &[py::Stmt], out: &mut Vec<(String, String)>) {
    for stmt in stmts {
        match stmt {
            py::Stmt::Assign(a) => {
                for target in &a.targets {
                    if let py::Expr::Attribute(attr) = target {
                        if is_self(&attr.value) {
                            let fname = attr.attr.to_string();
                            if !out.iter().any(|(n, _)| n == &fname) {
                                out.push((fname, "Any".to_string()));
                            }
                        }
                    }
                }
            }
            py::Stmt::AnnAssign(a) => {
                if let py::Expr::Attribute(attr) = &*a.target {
                    if is_self(&attr.value) {
                        let fname = attr.attr.to_string();
                        let type_ann = convert_annotation(&a.annotation);
                        if !out.iter().any(|(n, _)| n == &fname) {
                            out.push((fname, type_ann));
                        }
                    }
                }
            }
            py::Stmt::If(i) => {
                collect_self_fields(&i.body, out);
                collect_self_fields(&i.orelse, out);
            }
            py::Stmt::While(w) => collect_self_fields(&w.body, out),
            py::Stmt::For(f) => collect_self_fields(&f.body, out),
            py::Stmt::Try(t) => {
                collect_self_fields(&t.body, out);
                for h in &t.handlers {
                    let py::ExceptHandler::ExceptHandler(eh) = h;
                    collect_self_fields(&eh.body, out);
                }
                collect_self_fields(&t.finalbody, out);
            }
            _ => {}
        }
    }
}

/// `__init__` 引数リストからパラメータ名 → 型アノテーション文字列のマップを作る。
pub(crate) fn extract_param_types(args: &py::Arguments) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for arg in &args.args {
        if let Some(ann) = &arg.def.annotation {
            map.insert(arg.def.arg.to_string(), convert_annotation(ann));
        }
    }
    for arg in args.posonlyargs.iter().chain(args.kwonlyargs.iter()) {
        if let Some(ann) = &arg.def.annotation {
            map.insert(arg.def.arg.to_string(), convert_annotation(ann));
        }
    }
    map
}

// ---------------------------------------------------------------------------
// パラメータ変換
// ---------------------------------------------------------------------------

/// Python の引数リスト（`py::Arguments`）を tl の `Param` リストに変換する。
///
/// 順序は Python の宣言順（posonly → 通常 → vararg → kwonly）をそのまま保つ。
///
/// ⚠ **位置専用マーカ `/` と キーワード専用マーカ `*`（bare `*`）は無視して平坦化する**（項目 24）。
/// rustpython はどちらもマーカ自体をノードとして持たず、`posonlyargs` / `kwonlyargs` という
/// **区分**として表現するので、区分をまたいで 1 本の `Param` 列に並べるだけで済む。
/// 　- `def f(a, *, b)` → `Param[a, b]`。`f(1, b=2)` も `f(1, 2)` も通る。
/// 　- **意味の緩和**: Python はキーワード専用引数を位置渡しできない（`TypeError`）が、
/// 　  平坦化後の Arrow は位置渡しも受け付ける。Arrow 側に「位置渡し禁止」を表す
/// 　  `Param` のフラグが無いため。**受け入れる Python コードが広がる方向**なので許容
/// 　  （`/` も同様に「キーワード渡しも通る」方向へ緩む）。
///
/// ⚠ **デフォルト値は「定義時に 1 回」ではなく「呼び出しごと」に評価される**（項目 1）。
/// rustpython は `ArgWithDefault.default` として引数ごとにデフォルト式を持つので写すだけだが、
/// Arrow の `exec_fn_evaled` は毎回 `self.eval(expr)` する（`functions/execution.rs` の
/// `evaluated_defaults`）。Python は `def` の実行時に 1 回だけ評価して**その値を共有**する。
/// 　- リテラル（`0` / `"hi"` / `None` / `True`）は**完全に同じ**。実用上ほぼこれ。
/// 　- ⚠ **可変デフォルト**（`def f(x=[])`）だけ意味が違う: Python は同じリストを呼び出し間で
/// 　  共有する（有名な罠）が、Arrow は毎回新しいリストを作る。**Arrow 側が「普通に期待される」
/// 　  挙動**で、その罠に依存したコードだけが差を踏む。
/// 　- ⚠ 名前を参照するデフォルト（`def f(x=CONST)`）も、Arrow は呼び出し時に読み直す。
pub(crate) fn convert_params(
    args: &py::Arguments,
    filename: &str,
) -> Result<(Vec<Param>, std::collections::HashMap<String, ParamRename>), String> {
    let mut params: Vec<Param> = Vec::new();
    // 本体の識別子差し替え表（`*args` / `**kwargs` の Python 名 → Arrow 側の参照）。
    let mut renames: std::collections::HashMap<String, ParamRename> =
        std::collections::HashMap::new();

    // `/` より前（posonlyargs）と通常引数は区別せず同じ列に積む。
    for arg in args.posonlyargs.iter().chain(args.args.iter()) {
        let type_ann = arg.def.annotation.as_deref().map(convert_annotation);
        let default = arg
            .default
            .as_deref()
            .map(|e| convert_expr(e, filename))
            .transpose()?;
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
            default,
            variadic: false,
        });
    }


    // ★ 名前つき `*args` とキーワード専用引数の併用は**表現できない**ので明示エラー（A6）。
    //
    // Arrow の可変長パラメータは**必ず末尾**でなければならず、`Param` に
    // 「位置渡し禁止」のフラグが無い（項目 24）。そのため `def f(a, *rest, b)` を
    // 平坦化すると `[a, b, ...]` となり、**`b` が位置引数のスロットを占める**:
    //
    //   f(1, b=9)        → (1, 0, 9)  ◯ たまたま合う（位置引数が余らないため）
    //   f(1, 2, 3, b=9)  → TypeError: argument 'b' given twice
    //                      ↑ 2 が `b` に入ったあと、キーワードの `b` と衝突する
    //
    // ⚠ 「たまたま合う」呼び方があるのが厄介で、**呼び方によって通ったり壊れたり**する。
    //   `Param` にキーワード専用の概念を足さない限り直せないので、変換時に止める。
    // ⚠ **bare `*`（名前なし）は対象外**。vararg が無いので平坦化して問題なく、
    //   項目 24 で固定した 6 形はすべて通り続ける。
    if args.vararg.is_some() && !args.kwonlyargs.is_empty() {
        let names: Vec<&str> = args.kwonlyargs.iter().map(|a| a.def.arg.as_str()).collect();
        return Err(format!(
            "{filename}: keyword-only parameter(s) after a named `*{}` are not supported \
             (found: {}); Arrow requires the variadic parameter to be last and has no \
             keyword-only marker — use a bare `*` or move the parameter before `*{}`",
            args.vararg.as_ref().expect("checked by is_some").arg.as_str(),
            names.join(", "),
            args.vararg.as_ref().expect("checked by is_some").arg.as_str(),
        ));
    }

    // bare `*` より後ろのキーワード専用引数。通常引数として平坦化する（項目 24）。
    for arg in &args.kwonlyargs {
        let type_ann = arg.def.annotation.as_deref().map(convert_annotation);
        let default = arg
            .default
            .as_deref()
            .map(|e| convert_expr(e, filename))
            .transpose()?;
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
            default,
            variadic: false,
        });
    }

    // ★ `**kwargs` の番兵パラメータ（項目 7）。
    //   Arrow の識別子にできない名前なのでユーザのパラメータ名と衝突しない。
    //   `bind_args_relaxed` が余ったキーワード引数を集めて **`kwargs`** という名前で束縛する
    //   （**1 個も無くても空 dict**。Python では `kw` が常に存在するため）。
    //   ⚠ **可変長パラメータより前**に置くこと。`bind_args_relaxed` は可変長を末尾として扱う。
    if let Some(kwarg) = &args.kwarg {
        params.push(Param {
            name: PY_KWARGS_PARAM.to_string(),
            mutable: true,
            type_ann: Some("dict[str, Any]".to_string()),
            default: None,
            variadic: false,
        });
        // Python 側の名前（`**opts` など）を本体では `kwargs` として参照させる。
        renames.insert(kwarg.arg.to_string(), ParamRename::Kwargs);
    }

    // ★ `*args` の可変長パラメータ（項目 6）。
    //   Arrow の可変長は**名前を持たず**、本体からは `local::args` で参照する規約なので、
    //   Python 側の名前（`*xs` など）は本体で `local::args` に差し替える。
    //   ⚠ Arrow は可変長パラメータが**最後**であることを要求するので、必ず末尾に積む。
    if let Some(vararg) = &args.vararg {
        params.push(Param {
            name: "...".to_string(),
            mutable: true,
            type_ann: Some("list[Any]".to_string()),
            default: None,
            variadic: true,
        });
        renames.insert(vararg.arg.to_string(), ParamRename::LocalArgs);
    }

    Ok((params, renames))
}

