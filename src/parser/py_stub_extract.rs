// parser/py_stub_extract.rs — Python ソース（`.py` / `.pyi`）から型スタブを**行ベース**で抽出する。
//
// # なぜ独立したモジュールなのか
//
// 元は `parser/imports/mod.rs` の中にあったが、そこは `editor` feature では
// 拡張用の構文だけを読む実装（`imports_editor.rs`・削除済み）に差し替えられて**丸ごと消える**
// モジュールだった。そのため VS Code 拡張（wasm）からも使えるようにここへ出した。
// （今は拡張も `parser/imports/` を使う・editor_import_resolution_plan.md 3-2。）
//
// ⚠ ここは 1 行ずつ見るだけの近似抽出で、`python_converter`（＝本物の Python パーサ）は使わない
//    （`python_converter` が取りこぼした宣言を補う側）。⚠ 以前は「rustpython は wasm32 に載らない」
//    ことも理由だったが、載ることを確かめた（2-2・拡張の crate も `python_converter` を取り込む）
//    （`crates/arrow-frontend/Cargo.toml` の「ネイティブ依存を足さないこと」）。
//
// ⚠ CLI の `.pyi` 経路は **`python_converter` の結果をここで補完する**形（`py_modules.rs`）。
//    エディタは補完側だけを使うので、**取れる型は CLI の部分集合**になる。
//    ⇒ 型が少なく出ることはあっても、CLI と違う型が出ることはない
//    （`compare_wasm_frontend.ps1` が守る不変条件と同じ向き）。


/// Python ソースから型スタブを直接抽出する。
///
/// 規則（ffi_value_exchange_plan.md 7-27・利用者の決定）: **型を付けるのは注釈・スタブファイル（`.pyi`）があるものと、
/// クラスのコンストラクタ（`__init__`）の戻り値だけ**。それ以外は `Any`（Arrow では明示のダウンキャストを求める）。
///
/// - トップレベルの `def` / `async def` → 関数の宣言（`Stmt::FnDef`・`py_function`）。引数と戻り値は注釈どおり、注釈の
///   無いものは `Any`、既定値のある引数は省略できる。`*args` / `**kwargs` を持つ関数は引数を検査しない（7-30）
///   ⚠ 以前は `Stmt::Let(名前, "function->戻り値")` で仮引数を持たず、引数の数も型も検査されなかった。また注釈の無い
///     関数の `return` のリテラルから戻り値を推し量っていた（`return 1` → `int`。注釈でもスタブでもないので 7-27 で `Any` に）
/// - トップレベルの `class` → クラスの宣言（`Stmt::ClassDef`・`py_class_stub`）。コンストラクタで作った値はこのクラスの型
///   （`pc.Box`）で、メソッドの引数・戻り値と属性は注釈どおり、注釈の無いものは `Any`
///   ⚠ 以前は `function->Any`（作った値が `Any` で、属性もメソッドも使えなかった）。タスク 7.8 で「クラス名を戻り値の型に
///     すると、メンバーの表が無いので嘘の型になる」として `Any` にした経緯がある。今はメンバーの表（メソッドと属性）を
///     スタブに書くので、名前だけの型にはならない
///
/// ⚠⚠ **`Unresolved` にしてはいけない。** `type_check/stmt/check.rs` の `Stmt::Import` に
/// 先人の実測記録がある: `editor` ビルドで未知メンバを `Any` にすると
/// `OperationOnAny` の偽陽性が出る。2 つの「判らない」を混同しないこと:
/// **解決を試みていない**（editor・未読込）は `Unresolved`、
/// **解決を試みたが外部言語なので判らない**は `Any`。ここは後者。
/// - 複数行シグネチャ（ブラケット深さを追跡）に対応
/// - `_` で始まるプライベート関数は除外
pub(crate) fn extract_py_type_stubs(source: &str) -> Vec<crate::ast::Stmt> {
    let lines: Vec<&str> = source.lines().collect();
    let mut stmts: Vec<crate::ast::Stmt> = Vec::new();
    let mut i = 0usize;
    // 直前にトップレベルのデコレータ（`@dataclass` など）があったか
    let mut decorated = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let trimmed = trimmed.trim_end();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let line_indent = py_line_indent(line);
        if line_indent > 0 {
            // トップレベルの宣言の中身（`py_class_stub` / `py_block_end` が読み飛ばす）か、トップレベルの文の続き
            i += 1;
            continue;
        }
        if trimmed.starts_with('@') {
            decorated = true;
            i += 1;
            continue;
        }

        // トップレベルの class 定義（7-27）
        // ⚠ スタブ（`.ars`）を読んだ C# / JS のクラスは**通常の Arrow 宣言**として別経路で登録されるので、ここを通らない。
        if py_parse_class_name(trimmed).is_some() {
            let (stmt, next) = py_class_stub(&lines, i, decorated);
            stmts.push(stmt);
            decorated = false;
            i = next;
            continue;
        }

        // 関数定義（トップレベルのみ対象）
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            decorated = false;
            if let Some((name, ret_ann, params_text, body_line, func_indent)) =
                py_parse_func_def(&lines, i)
            {
                if !name.starts_with('_') {
                    // 注釈の無い戻り値・引数は `Any`（7-27・7-30・注釈かスタブファイルがあるものだけに型を付ける）
                    let arrow_ret = match ret_ann {
                        Some(ann) => py_type_to_arrow(py_unquote(&ann)),
                        None => "Any".to_string(),
                    };
                    let params = py_params(&params_text, false).unwrap_or_else(py_open_params);
                    stmts.push(py_function(&name, params, arrow_ret));
                }
                i = py_block_end(&lines, body_line, func_indent);
                continue;
            }
        }

        decorated = false;
        i += 1;
    }

    py_unknown_types_to_any(&mut stmts);
    stmts
}

/// 型スタブの型の中で、**このファイルがクラスとして宣言していない大文字始まりの名前**を `Any` にする（7-30）。
/// Python の注釈は型の別名（`numpy.typing.ArrayLike`）やほかのモジュールのクラスを名前で書けるが、スタブはその中身を
/// 知らないので、Arrow の型としては検査できない（利用者の決定: 型を付けるのは注釈・スタブで型が分かるものだけ）。
/// ⚠ 以前は大文字始まりの名前をそのままクラスの型にしていたので、引数に型を付けた 7-30 で `plt.pie([..])` が
///   `expects 'ArrayLike' but got 'list'` の偽の静的エラーになった。
/// `.py` の抽出（`extract_py_type_stubs`）と `.pyi` の変換（`build_py_type_body`）の両方の結果に使う。
pub(crate) fn py_unknown_types_to_any(stmts: &mut [crate::ast::Stmt]) {
    use crate::ast::Stmt;
    let known: std::collections::HashSet<String> = stmts
        .iter()
        .filter_map(|s| if let Stmt::ClassDef { name, .. } = s { Some(name.clone()) } else { None })
        .collect();
    let fix = |t: &mut String| *t = py_type_names_to_any(t, &known);
    let fix_fn = |params: &mut Vec<crate::ast::Param>, ret: &mut Option<String>| {
        for p in params.iter_mut() {
            if let Some(t) = p.type_ann.as_mut() {
                fix(t);
            }
        }
        if let Some(t) = ret.as_mut() {
            fix(t);
        }
    };
    for st in stmts.iter_mut() {
        match st {
            Stmt::FnDef { params, return_type, .. } => fix_fn(params, return_type),
            Stmt::ClassDef { body, .. } => {
                for m in body.iter_mut() {
                    match m {
                        Stmt::FnDef { params, return_type, .. } => fix_fn(params, return_type),
                        Stmt::Field { type_ann, .. } => fix(type_ann),
                        _ => {}
                    }
                }
            }
            Stmt::Let(_, Some(t), _, _) => fix(t),
            _ => {}
        }
    }
}

/// 型の字句の中の、知らない大文字始まりの名前を `Any` にする（`py_unknown_types_to_any`）。Arrow の型の構成子
/// （`Union` / `Option` / `Any` / `None` …）と `known` の名前は残す。
fn py_type_names_to_any(ty: &str, known: &std::collections::HashSet<String>) -> String {
    const ARROW_TYPE_NAMES: &[&str] = &["Any", "None", "Union", "Option", "Intersection", "Result", "Self", "Undefined"];
    let mut out = String::with_capacity(ty.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            let keep = !word.starts_with(|c: char| c.is_ascii_uppercase())
                || ARROW_TYPE_NAMES.contains(&word.as_str())
                || known.contains(word.as_str());
            out.push_str(if keep { word } else { "Any" });
            word.clear();
        }
    };
    for c in ty.chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// `start` から読んで、インデントが `indent` 以下の（空でもコメントでもない）行の位置。ブロックの終わり。
fn py_block_end(lines: &[&str], start: usize, indent: usize) -> usize {
    let mut i = start;
    while i < lines.len() {
        let t = lines[i].trim();
        if !t.is_empty() && !t.starts_with('#') && py_line_indent(lines[i]) <= indent {
            break;
        }
        i += 1;
    }
    i
}

/// Python のクラスに付ける印（クラスの宣言のデコレータ）: メンバーの顔ぶれが分からない（`members_unresolved`）。
/// 型検査は外部言語のクラスにデコレータがあるとメンバー存在検査を黙らせる（`TypeRegistryBuilder` の `foreign_depth`）。
const PY_OPEN_MEMBERS_MARK: &str = "py-open-members: ";

/// トップレベルの `class` を型スタブのクラスの宣言にする（ffi_value_exchange_plan.md 7-27）。`start` は `class` の行。
/// 戻り値は（宣言, 次に見る行）。
///
/// - コンストラクタ: `__init__` の引数（`self` を除く）。作った値の型はこのクラス（利用者の決定: 型を付けるのは `__init__` の
///   戻り値だけ）。`__init__` が無いクラスは引数なし（`object.__init__`）、基底が分からないクラスは引数を問わない
/// - メソッド: 引数と戻り値は注釈どおり、注釈の無いものは `Any`。`@staticmethod` / `@classmethod` は `self` / `cls` を持たない
/// - 属性: クラス本体の `name: T` / `name = …` と、メソッドの中の `self.name: T = …` / `self.name = …`。注釈の無いものは `Any`。
///   `@property` も属性（戻り値の注釈の型）
/// - 基底（`object` 以外）が同じファイルのクラスならそのメンバーも持つ。ほかのファイル・ライブラリのクラスを継承する
///   クラスと、デコレータ（`@dataclass` など）の付いたクラスは、メンバーの顔ぶれが分からないので開いておく
///   （書いたメンバーは型が付き、ほかの名前は検査しない）
fn py_class_stub(lines: &[&str], start: usize, decorated: bool) -> (crate::ast::Stmt, usize) {
    use crate::ast::{Accessibility, Expr, FieldKind, Stmt};
    let header = py_strip_comment(lines[start]).trim();
    let name = py_parse_class_name(header).unwrap_or_default();
    let end = py_block_end(lines, start + 1, 0);
    let bases: Vec<String> = match (header.find('('), header.rfind(')')) {
        (Some(o), Some(c)) if o < c => split_top_level(&header[o + 1..c], ',')
            .into_iter()
            .filter(|b| !b.is_empty() && *b != "object" && !b.contains('='))
            .map(|b| b.to_string())
            .collect(),
        _ => Vec::new(),
    };
    let members = py_class_members(lines, start, end);
    // 同じファイルの基底のメンバー（先に書く。派生で同じ名前があれば派生が勝つ）
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut methods: Vec<Stmt> = Vec::new();
    let mut open = decorated;
    for base in &bases {
        match py_find_class(lines, base) {
            Some(b) if b != start => {
                let base_end = py_block_end(lines, b + 1, 0);
                let bm = py_class_members(lines, b, base_end);
                for f in bm.fields {
                    py_add_field(&mut fields, f.0, f.1);
                }
                methods.extend(bm.methods.into_iter().filter(|m| !matches!(m, Stmt::FnDef { name, .. } if name == "__init__")));
            }
            _ => open = true,
        }
    }
    for f in members.fields {
        py_add_field(&mut fields, f.0, f.1);
    }
    for m in members.methods {
        let mname = match &m { Stmt::FnDef { name, .. } => name.clone(), _ => String::new() };
        methods.retain(|x| !matches!(x, Stmt::FnDef { name, .. } if *name == mname));
        methods.push(m);
    }
    if !methods.iter().any(|m| matches!(m, Stmt::FnDef { name, .. } if name == "__init__")) {
        // `__init__` が無い: 基底が分からなければ引数を問わない、そうでなければ引数なし
        let params = if open { py_open_params() } else { Vec::new() };
        methods.push(py_method("__init__", params, Some("None".to_string()), false));
    }
    let mut body: Vec<Stmt> = fields
        .into_iter()
        .map(|(fname, ty)| Stmt::Field {
            src: None,
            name: fname,
            kind: FieldKind::Mut,
            type_ann: ty,
            default: None,
            access: Accessibility::Public,
        })
        .collect();
    body.extend(methods);
    let decorators = if open {
        vec![Expr::Str(
            format!("{PY_OPEN_MEMBERS_MARK}the class is decorated or inherits from a class outside this file").into(),
        )]
    } else {
        Vec::new()
    };
    (
        Stmt::ClassDef {
            src: None,
            name,
            template_params: vec![],
            bases: vec![],
            base_args: vec![],
            decorators,
            body,
        },
        end,
    )
}

/// 同じファイルのトップレベルの `class name` の行。
fn py_find_class(lines: &[&str], name: &str) -> Option<usize> {
    lines.iter().position(|l| py_line_indent(l) == 0 && py_parse_class_name(l.trim()).as_deref() == Some(name))
}

/// クラスの本体から集めたメンバー（`py_class_members`）。
struct PyClassMembers {
    /// （名前, 型）。注釈の無いものは `Any`
    fields: Vec<(String, String)>,
    methods: Vec<crate::ast::Stmt>,
}

/// 同じ名前の属性は、注釈のあるもの（`Any` でない型）を勝たせる。
fn py_add_field(fields: &mut Vec<(String, String)>, name: String, ty: String) {
    match fields.iter_mut().find(|(n, _)| *n == name) {
        Some(slot) => {
            if slot.1 == "Any" {
                slot.1 = ty;
            }
        }
        None => fields.push((name, ty)),
    }
}

/// `class` の行 `start` から `end` までの本体のメソッドと属性。
fn py_class_members(lines: &[&str], start: usize, end: usize) -> PyClassMembers {
    let mut out = PyClassMembers { fields: Vec::new(), methods: Vec::new() };
    let body_indent = lines[start + 1..end]
        .iter()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .map(|l| py_line_indent(l))
        .unwrap_or(4);
    let mut decos: Vec<String> = Vec::new();
    let mut i = start + 1;
    while i < end {
        let line = lines[i];
        let t = py_strip_comment(line).trim();
        if t.is_empty() || py_line_indent(line) != body_indent {
            i += 1;
            continue;
        }
        if let Some(d) = t.strip_prefix('@') {
            decos.push(d.trim().to_string());
            i += 1;
            continue;
        }
        if t.starts_with("def ") || t.starts_with("async def ") {
            if let Some((mname, ret, params, body_line, _)) = py_parse_func_def(lines, i) {
                let body_end = py_block_end(lines, body_line, body_indent);
                for bl in &lines[body_line..body_end] {
                    for (n, ty) in py_self_attrs(py_strip_comment(bl).trim()) {
                        py_add_field(&mut out.fields, n, ty);
                    }
                }
                let ret_ty = ret.as_deref().map(|r| py_type_to_arrow(py_unquote(r)));
                let is_static = decos.iter().any(|d| d == "staticmethod" || d == "classmethod");
                if decos.iter().any(|d| d == "property" || d == "functools.cached_property" || d == "cached_property") {
                    py_add_field(&mut out.fields, mname, ret_ty.unwrap_or_else(|| "Any".to_string()));
                } else if !decos.iter().any(|d| d.ends_with(".setter") || d.ends_with(".deleter")) {
                    let params = py_params(&params, !is_static).unwrap_or_else(py_open_params);
                    let ret_ty = if mname == "__init__" { Some("None".to_string()) } else { Some(ret_ty.unwrap_or_else(|| "Any".to_string())) };
                    out.methods.retain(|m| !matches!(m, crate::ast::Stmt::FnDef { name, .. } if *name == mname));
                    out.methods.push(py_method(&mname, params, ret_ty, is_static));
                }
                decos.clear();
                i = body_end;
                continue;
            }
        }
        if t.starts_with("class ") {
            // 入れ子のクラスは読み飛ばす（Arrow から `Outer.Inner` で使う形はスタブに書かない）
            decos.clear();
            i = py_block_end(lines, i + 1, body_indent);
            continue;
        }
        // クラス本体の属性: `name: T = …` / `name: T` / `name = …`
        if let Some((n, ty)) = py_attr_decl(t) {
            py_add_field(&mut out.fields, n, ty);
        }
        decos.clear();
        i += 1;
    }
    out
}

/// クラス本体の 1 行が属性の宣言なら（名前, 型）。`name: T = …` / `name: T` は T、`name = …` は `Any`。
fn py_attr_decl(t: &str) -> Option<(String, String)> {
    let ident_end = t.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(t.len());
    let name = &t[..ident_end];
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
        return None;
    }
    if matches!(name, "pass" | "return" | "if" | "for" | "while" | "with" | "try" | "import" | "from" | "del" | "assert" | "raise") {
        return None;
    }
    let rest = t[ident_end..].trim_start();
    if let Some(ann_rest) = rest.strip_prefix(':') {
        let ann = match split_top_level(ann_rest, '=').first() {
            Some(a) => a.trim().to_string(),
            None => return None,
        };
        if ann.is_empty() {
            return None;
        }
        return Some((name.to_string(), py_type_to_arrow(py_unquote(&ann))));
    }
    if rest.starts_with('=') && !rest.starts_with("==") {
        return Some((name.to_string(), "Any".to_string()));
    }
    None
}

/// メソッドの本体の 1 行の `self.name: T = …` / `self.name = …`（`self.a, self.b = …` も）の（名前, 型）。
fn py_self_attrs(t: &str) -> Vec<(String, String)> {
    if !t.starts_with("self.") {
        return Vec::new();
    }
    // 最初の `=`（`==` / `<=` / `>=` / `!=` / `+=` などではない）の前が代入の対象
    let bytes = t.as_bytes();
    let mut eq = None;
    let mut depth = 0i32;
    for (k, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = if k > 0 { bytes[k - 1] } else { b' ' };
                let next = bytes.get(k + 1).copied().unwrap_or(b' ');
                if next != b'=' && !b"=<>!+-*/%&|^@:".contains(&prev) {
                    eq = Some(k);
                    break;
                }
                if next == b'=' {
                    return Vec::new();
                }
            }
            _ => {}
        }
    }
    let Some(eq) = eq else { return Vec::new() };
    let target = t[..eq].trim();
    // `self.name: T`（注釈つきは 1 つだけ）
    if let Some((lhs, ann)) = target.split_once(':') {
        let n = lhs.trim().strip_prefix("self.").unwrap_or("");
        if !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return vec![(n.to_string(), py_type_to_arrow(py_unquote(ann.trim())))];
        }
        return Vec::new();
    }
    split_top_level(target, ',')
        .into_iter()
        .filter_map(|part| {
            let n = part.trim().strip_prefix("self.")?;
            (!n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| (n.to_string(), "Any".to_string()))
        })
        .collect()
}

/// 注釈の文字列（前方参照の `"Box"`）の引用符を外す。
fn py_unquote(a: &str) -> &str {
    let a = a.trim();
    a.strip_prefix('"').and_then(|x| x.strip_suffix('"'))
        .or_else(|| a.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
        .unwrap_or(a)
}

/// 引数を問わないシグネチャ（`*args, **kwargs`）。
fn py_open_params() -> Vec<crate::ast::Param> {
    vec![
        crate::ast::Param {
            name: crate::ast::PY_KWARGS_PARAM.to_string(),
            mutable: false,
            type_ann: Some("dict[str, Any]".to_string()),
            default: None,
            variadic: false,
        },
        crate::ast::Param { name: "...".to_string(), mutable: false, type_ann: Some("list[Any]".to_string()), default: None, variadic: true },
    ]
}

/// Python の引数の並び（`(` と `)` の間）を Arrow の仮引数にする。`with_self` なら先頭（`self` / `cls`）を外す。
/// 注釈の無い引数は `Any`、既定値のある引数は省略できる。`*args` は可変長（末尾）、`**kwargs` は `PY_KWARGS_PARAM`。
/// 読めない形なら `None`（呼び出し側は引数を問わないシグネチャにする）。
fn py_params(text: &str, with_self: bool) -> Option<Vec<crate::ast::Param>> {
    use crate::ast::Param;
    let mut params: Vec<Param> = Vec::new();
    let mut kwargs: Option<Param> = None;
    let mut varargs: Option<Param> = None;
    let mut first = true;
    for part in split_top_level(text, ',') {
        let part = part.trim();
        if part.is_empty() || part == "/" || part == "*" {
            continue;
        }
        if first && with_self {
            first = false;
            continue;
        }
        first = false;
        if part.starts_with("**") {
            kwargs = Some(Param {
                name: crate::ast::PY_KWARGS_PARAM.to_string(),
                mutable: false,
                type_ann: Some("dict[str, Any]".to_string()),
                default: None,
                variadic: false,
            });
            continue;
        }
        if part.starts_with('*') {
            varargs = Some(Param { name: "...".to_string(), mutable: false, type_ann: Some("list[Any]".to_string()), default: None, variadic: true });
            continue;
        }
        let pieces = split_top_level(part, '=');
        let decl = pieces.first()?.trim();
        let has_default = pieces.len() > 1;
        let (pname, ann) = match decl.split_once(':') {
            Some((n, a)) => (n.trim(), Some(a.trim())),
            None => (decl, None),
        };
        if pname.is_empty() || !pname.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let ty = ann.map(|a| py_param_type(&py_type_to_arrow(py_unquote(a)))).unwrap_or_else(|| "Any".to_string());
        // 既定値の式は型スタブでは使わない（省略できることだけを表す）。型の合わない `None` を書くと型検査が既定値を
        // 疑うので、`Any` の値として `None` を置く（`Option` でない注釈なら型を `Any` に広げない）
        params.push(Param {
            name: pname.to_string(),
            mutable: false,
            type_ann: Some(ty),
            default: has_default.then_some(crate::ast::Expr::None),
            variadic: false,
        });
    }
    params.extend(kwargs);
    params.extend(varargs);
    Some(params)
}

/// 引数の注釈の型（Arrow の綴り）を、Python の型の規則で受けられる型にする（7-30）: PEP 484 の数の塔で、`float` と注釈した
/// 引数は `int` も受ける（`math.sqrt(2)` は Python では正しい）。⚠ 戻り値には使わない（`-> float` は `float` を返す）。
pub(crate) fn py_param_type(arrow_ty: &str) -> String {
    match arrow_ty {
        "float" => "Union[int, float]".to_string(),
        other => other.to_string(),
    }
}

/// 型スタブのトップレベルの関数の宣言（本体は空・7-30）。
fn py_function(name: &str, params: Vec<crate::ast::Param>, ret: String) -> crate::ast::Stmt {
    crate::ast::Stmt::FnDef {
        src: None,
        name: name.to_string(),
        template_params: vec![],
        params,
        return_type: Some(ret),
        body: vec![],
        is_abstract: false,
        is_static: false,
        is_class_method: false,
        decorators: vec![],
        access: crate::ast::Accessibility::Public,
    }
}

/// 型スタブのメソッドの宣言（本体は空・`self` は不変）。
fn py_method(name: &str, params: Vec<crate::ast::Param>, ret: Option<String>, is_static: bool) -> crate::ast::Stmt {
    use crate::ast::{Accessibility, Param, Stmt};
    let mut all = Vec::new();
    if !is_static {
        all.push(Param { name: "self".to_string(), mutable: false, type_ann: None, default: None, variadic: false });
    }
    all.extend(params);
    Stmt::FnDef {
        src: None,
        name: name.to_string(),
        template_params: vec![],
        params: all,
        return_type: ret,
        body: vec![],
        is_abstract: false,
        is_static,
        is_class_method: false,
        decorators: vec![],
        access: Accessibility::Public,
    }
}

/// 行の先頭インデント幅（スペース/タブ数）を返す。
fn py_line_indent(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// `class Foo` や `class Foo(Base):` からクラス名を取り出す。
fn py_parse_class_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("class ")?;
    let name_end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() { None } else { Some(name.to_string()) }
}

/// `def` / `async def` をパースして `(名前, 戻り値アノテーション, 引数の並び, body_start_idx, func_indent)` を返す。
/// 複数行シグネチャはブラケット深さを追跡して結合する（最大 40 行）。
fn py_parse_func_def(
    lines: &[&str],
    start: usize,
) -> Option<(String, Option<String>, String, usize, usize)> {
    let start_line = lines[start];
    let func_indent = py_line_indent(start_line);
    let trimmed = start_line.trim_start();

    // 関数名を取り出す
    let after_def = trimmed
        .strip_prefix("async def ")
        .or_else(|| trimmed.strip_prefix("def "))?;
    let paren = after_def.find('(')?;
    let name = after_def[..paren].trim().to_string();
    if name.is_empty() { return None; }

    // 複数行シグネチャを1行に結合（括弧が閉じるまで）
    let mut depth = 0i32;
    let mut combined = String::new();
    let mut body_line = start + 1;
    for (j, &line) in lines[start..].iter().enumerate().take(40) {
        let raw = py_strip_comment(line);
        if j > 0 { combined.push(' '); }
        combined.push_str(raw.trim());
        for ch in raw.chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            body_line = start + j + 1;
            break;
        }
    }

    // 閉じ括弧の位置を探す
    let open = combined.find('(')?;
    let mut pd = 0i32;
    let mut close = None;
    for (k, ch) in combined[open..].char_indices() {
        match ch {
            '(' => pd += 1,
            ')' => {
                pd -= 1;
                if pd <= 0 { close = Some(open + k); break; }
            }
            _ => {}
        }
    }
    let close = close?;
    let params = combined[open + 1..close].to_string();
    let after = combined[close + 1..].trim();

    // `-> ReturnType:` の形式から戻り値アノテーションを取り出す。
    //
    // ⚠⚠ **シグネチャと同じ行に本体が続く形がある。** `.pyi` の標準形は
    //    `def sqrt(x: float) -> float: ...` で、以前は行末の `:` だけを
    //    `trim_end_matches(':')` で削っていたため、戻り値型が `"float: ..."` になり
    //    `py_type_to_arrow` の catch-all で **`Any` に落ちていた**
    //    （＝同梱スタブを整備しても型予測が効かない）。
    // ⇒ **深さ 0 の `:` まで**を型として切り出す。`-> dict[str, int]: ...` の
    //    ように型の中に `:` が入る形があるので、行末からではなく前から探す。
    let ret_ann = if let Some(s) = after.strip_prefix("->") {
        let s = s.trim();
        let mut depth = 0i32;
        let mut end = s.len();
        for (k, ch) in s.char_indices() {
            match ch {
                '[' | '(' | '{' => depth += 1,
                ']' | ')' | '}' => depth -= 1,
                ':' if depth <= 0 => {
                    end = k;
                    break;
                }
                _ => {}
            }
        }
        let s = s[..end].trim();
        if s.is_empty() { None } else { Some(s.to_string()) }
    } else {
        None
    };

    Some((name, ret_ann, params, body_line, func_indent))
}

/// 行中のコメント（`#` 以降）を除去する簡易実装（文字列リテラル内の `#` は無視）。
fn py_strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut str_char = b' ';
    for (i, &b) in line.as_bytes().iter().enumerate() {
        if in_str {
            if b == str_char { in_str = false; }
        } else {
            if b == b'#' { return &line[..i]; }
            if b == b'"' || b == b'\'' { in_str = true; str_char = b; }
        }
    }
    line
}

/// Python 型アノテーション文字列を Arrow の `from_ann` が解釈できる文字列に変換する。
///
/// ⚠ 角括弧の中は入れ子を数えて区切る（ffi_value_exchange_plan.md 7-8）。以前は前方一致で処理していて、`dict[K, V]` /
///   `tuple[...]` は要素型を捨て（`set[tuple[int, int]]` → `set[tuple]`）、`frozenset[T]` は `Any`、`Union[...]` は中を変換
///   しないまま、角括弧を含む `X | None` は `Any` になっていた。要素型が落ちると境界の照合（`ffi_boundary`）が要素を見ない。
/// ⚠ Arrow が表せない型（`Sequence[T]` / `Callable[...]` / `Literal[...]` / 可変長の `tuple[int, ...]` の要素など）は
///   検査しない側（`Any` / 素の `tuple`）に倒す（Python の注釈は強制されないので、狭く読むと正しい値を拒む）。
fn py_type_to_arrow(ann: &str) -> String {
    let a = ann.trim();
    // PEP 604: 最上位（角括弧の外）の `|`
    let alternatives = split_top_level(a, '|');
    if alternatives.len() > 1 {
        return union_of(alternatives.iter().map(|t| py_type_to_arrow(t)).collect());
    }
    // ジェネリクス `Base[args]`（`typing.List[int]` の `typing.` は落とす）
    if let Some((base, inner)) = a.strip_suffix(']').and_then(|s| s.split_once('[')) {
        let base = base.trim().rsplit('.').next().unwrap_or("").trim();
        let raw_args = split_top_level(inner, ',');
        let args: Vec<String> = raw_args.iter().map(|t| py_type_to_arrow(t)).collect();
        return match (base, args.as_slice()) {
            ("Optional", [t]) => union_of(vec![t.clone(), "None".to_string()]),
            ("Union", ts) if !ts.is_empty() => union_of(ts.to_vec()),
            ("List" | "list", [t]) => format!("list[{t}]"),
            // frozenset も実行時は Arrow の set で届く（`py_to_tl`・2-6）
            ("Set" | "set" | "FrozenSet" | "frozenset", [t]) => format!("set[{t}]"),
            ("Dict" | "dict", [k, v]) => format!("dict[{k}, {v}]"),
            // 可変長（`tuple[int, ...]`）と空（`tuple[()]`）は要素の並びを Arrow で書けないので素の `tuple`（形だけ照合）
            ("Tuple" | "tuple", _) if raw_args.iter().any(|t| matches!(t.trim(), "..." | "()")) => "tuple".to_string(),
            ("Tuple" | "tuple", ts) if !ts.is_empty() => format!("tuple[{}]", ts.join(", ")),
            // 知らないジェネリクス（`Sequence[T]` / `Callable[...]` / 利用者のクラス `Box[int]` など）は検査しない
            _ => "Any".to_string(),
        };
    }
    match a {
        "str" | "int" | "float" | "bool" | "None" | "Any" | "bytes"
        | "list" | "dict" | "tuple" | "set" => a.to_string(),
        "frozenset" => "set".to_string(),
        // Python のバイト列はどれも Arrow の bytes として受け取る（ffi_value_exchange_plan.md 5-3）。要素が 1 バイトでない
        // memoryview は Python のオブジェクトのまま届き、境界の照合で FfiTypeError になる
        "bytearray" | "memoryview" => "bytes".to_string(),
        // 大文字始まりのクラス名はそのまま（NamedInstance として解釈される）
        _ if a.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && a.chars().all(|c| c.is_alphanumeric() || c == '_') =>
        {
            a.to_string()
        }
        // 不明・複雑な型 → Any
        _ => "Any".to_string(),
    }
}

/// 型の選択肢を Arrow の型にする: `None` との 2 択は `Option[T]`、`Any` を含めば `Any`（どの値も入る）。
fn union_of(types: Vec<String>) -> String {
    if types.iter().any(|t| t == "Any") {
        return "Any".to_string();
    }
    let others: Vec<&String> = types.iter().filter(|t| *t != "None").collect();
    if types.len() == 2 && others.len() == 1 {
        return format!("Option[{}]", others[0]);
    }
    format!("Union[{}]", types.join(", "))
}

/// `sep` で区切る。角括弧・丸括弧の中の `sep` では区切らない（`dict[str, tuple[int, int]]` の中の `,` など）。
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(s[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_parts(stmts: &[crate::ast::Stmt], class: &str) -> (Vec<(String, String)>, Vec<(String, Vec<String>, String)>, bool) {
        use crate::ast::Stmt;
        let Some(Stmt::ClassDef { body, decorators, .. }) =
            stmts.iter().find(|s| matches!(s, Stmt::ClassDef { name, .. } if name == class))
        else {
            panic!("no class {class} in {stmts:?}")
        };
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        for st in body {
            match st {
                Stmt::Field { name, type_ann, .. } => fields.push((name.clone(), type_ann.clone())),
                Stmt::FnDef { name, params, return_type, .. } => methods.push((
                    name.clone(),
                    params.iter().map(|p| format!("{}:{}", p.name, p.type_ann.clone().unwrap_or_default())).collect(),
                    return_type.clone().unwrap_or_default(),
                )),
                _ => {}
            }
        }
        (fields, methods, !decorators.is_empty())
    }

    /// クラスは宣言にする: コンストラクタの引数・注釈どおりのメンバー・注釈の無いものは `Any`（7-27・利用者の決定）
    #[test]
    fn classes_become_declarations() {
        let src = "class Counter:\n    label: str = 'c'\n    def __init__(self, start: int, step=1) -> None:\n        self.value: int = start\n        self.step = step\n        self.a, self.b = 1, 2\n        self.value += 1\n    def bump(self) -> int:\n        return 1\n    def describe(self):\n        return ''\n    @property\n    def count(self) -> int:\n        return 0\n    @staticmethod\n    def make(n: int) -> \"Counter\":\n        return Counter(n)\n\ndef untyped(x):\n    return 1\n";
        let stmts = extract_py_type_stubs(src);
        let (fields, methods, open) = class_parts(&stmts, "Counter");
        assert_eq!(
            fields,
            vec![
                ("label".to_string(), "str".to_string()),
                ("value".to_string(), "int".to_string()),
                ("step".to_string(), "Any".to_string()),
                ("a".to_string(), "Any".to_string()),
                ("b".to_string(), "Any".to_string()),
                ("count".to_string(), "int".to_string()),
            ]
        );
        let m = |n: &str| methods.iter().find(|(name, _, _)| name == n).cloned().unwrap_or_else(|| panic!("no method {n}"));
        assert_eq!(m("__init__"), ("__init__".to_string(), vec!["self:".to_string(), "start:int".to_string(), "step:Any".to_string()], "None".to_string()));
        assert_eq!(m("bump").2, "int");
        assert_eq!(m("describe").2, "Any", "no annotation -> Any");
        assert_eq!(m("make"), ("make".to_string(), vec!["n:int".to_string()], "Counter".to_string()), "static: no self, quoted return");
        assert!(!open, "a class without bases or decorators has a closed member set");
        // 注釈の無いトップレベルの関数は Any（以前は `return 1` から int と推し量っていた）。引数も Any（7-30）
        assert!(stmts.iter().any(|s| matches!(s, crate::ast::Stmt::FnDef { name, params, return_type: Some(t), .. }
            if name == "untyped" && t == "Any" && params.len() == 1 && params[0].type_ann.as_deref() == Some("Any"))));
    }

    /// 基底: 同じファイルのクラスならメンバーを持つ。ほかのファイルのクラスを継承する・デコレータの付いたクラスは開いておく
    #[test]
    fn class_bases_and_decorators() {
        let src = "class Base:\n    def hello(self) -> str:\n        return ''\n\nclass Child(Base):\n    def own(self) -> int:\n        return 0\n\nclass Foreign(somelib.Thing):\n    pass\n\n@dataclass\nclass Data:\n    x: int\n";
        let stmts = extract_py_type_stubs(src);
        let (_, methods, open) = class_parts(&stmts, "Child");
        let names: Vec<&str> = methods.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"hello") && names.contains(&"own"), "{names:?}");
        assert!(!open);
        let (_, methods, open) = class_parts(&stmts, "Foreign");
        assert!(open, "a base outside the file: members are open");
        assert_eq!(methods[0].1, vec!["self:".to_string(), "**kwargs:dict[str, Any]".to_string(), "...:list[Any]".to_string()], "and the constructor takes anything");
        let (fields, _, open) = class_parts(&stmts, "Data");
        assert!(open, "a decorated class (dataclass adds members): open");
        assert_eq!(fields, vec![("x".to_string(), "int".to_string())]);
    }

    /// このファイルが宣言していないクラス・型の別名の名前は `Any`（7-30）
    #[test]
    fn unknown_type_names_become_any() {
        let src = "class Box:
    pass

def f(x: ArrayLike, b: Box, xs: list[Other] | None) -> Box:
    return b
";
        let stmts = extract_py_type_stubs(src);
        let Some(crate::ast::Stmt::FnDef { params, return_type, .. }) =
            stmts.iter().find(|s| matches!(s, crate::ast::Stmt::FnDef { name, .. } if name == "f"))
        else {
            panic!("no f")
        };
        let types: Vec<&str> = params.iter().map(|p| p.type_ann.as_deref().unwrap_or("")).collect();
        assert_eq!(types, vec!["Any", "Box", "Option[list[Any]]"]);
        assert_eq!(return_type.as_deref(), Some("Box"));
    }

    /// Python のバイト列の注釈はどれも `bytes`（ffi_value_exchange_plan.md 5-3）。
    #[test]
    fn byte_sequences_are_bytes() {
        assert_eq!(py_type_to_arrow("bytes"), "bytes");
        assert_eq!(py_type_to_arrow("bytearray"), "bytes");
        assert_eq!(py_type_to_arrow("Optional[memoryview]"), "Option[bytes]");
        assert_eq!(py_type_to_arrow("list[bytearray]"), "list[bytes]");
    }

    /// 要素型を落とさない（7-8）。以前は `dict[K, V]` → `dict`・`set[tuple[int, int]]` → `set[tuple]`・`frozenset[T]` → `Any`・
    /// `Union[List[int], None]` は中を変換しないまま・`list[int] | None` → `Any` だった。
    #[test]
    fn element_types_are_kept() {
        assert_eq!(py_type_to_arrow("set[tuple[int, int]]"), "set[tuple[int, int]]");
        assert_eq!(py_type_to_arrow("frozenset[str]"), "set[str]");
        assert_eq!(py_type_to_arrow("FrozenSet[int]"), "set[int]");
        assert_eq!(py_type_to_arrow("dict[str, list[int]]"), "dict[str, list[int]]");
        assert_eq!(py_type_to_arrow("Dict[str, Tuple[int, float]]"), "dict[str, tuple[int, float]]");
        assert_eq!(py_type_to_arrow("Union[List[int], None]"), "Option[list[int]]");
        assert_eq!(py_type_to_arrow("Union[int, str, None]"), "Union[int, str, None]");
        assert_eq!(py_type_to_arrow("list[int] | None"), "Option[list[int]]");
        assert_eq!(py_type_to_arrow("int | str"), "Union[int, str]");
        assert_eq!(py_type_to_arrow("typing.Optional[typing.List[str]]"), "Option[list[str]]");
        // Arrow が表せないものは検査しない側
        assert_eq!(py_type_to_arrow("tuple[int, ...]"), "tuple");
        assert_eq!(py_type_to_arrow("Sequence[int]"), "Any");
        assert_eq!(py_type_to_arrow("Callable[[int], str]"), "Any");
        assert_eq!(py_type_to_arrow("Optional[Any]"), "Any");
        assert_eq!(py_type_to_arrow("Person"), "Person");
    }
}
