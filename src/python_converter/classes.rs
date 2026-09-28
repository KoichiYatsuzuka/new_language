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
    // 変換しない文を明示エラーにする（黙って捨てない・10-18）。
    let unsupported = |what: &str| -> Result<(), String> {
        Err(format!(
            "{filename}: {what} in the body of class '{class_name}' is not supported \
             (only methods, class attributes and nested classes are converted)"
        ))
    };

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
                let dec = convert_decorators(
                    &f.decorator_list,
                    filename,
                    &format!("method '{}.{}'", class_name, f.name.as_str()),
                    true,
                )?;
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
                        name: f.name.to_string(),
                        template_params: vec![],
                        params,
                        yield_type: extract_yield_type(f.returns.as_deref()),
                        body,
                        access: crate::ast::Accessibility::Public,
                    });
                    continue;
                }
                methods.push(Stmt::FnDef { src: None,
                    name: f.name.to_string(),
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
    Ok(hoisted)
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

