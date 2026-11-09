// parser/py_stub_extract.rs — Python ソース（`.py` / `.pyi`）から型スタブを**行ベース**で抽出する。
//
// # なぜ独立したモジュールなのか
//
// 元は `parser/imports/mod.rs` の中にあったが、そこは `editor` feature では
// 拡張用の構文だけを読む実装（`imports_editor.rs`・削除済み）に差し替えられて**丸ごと消える**
// モジュールだった。そのため VS Code 拡張（wasm）からも使えるようにここへ出した。
// （今は拡張も `parser/imports/` を使う・editor_import_resolution_plan.md 3-2。）
//
// ⚠ ここは 1 行ずつ見るだけの近似抽出で、本物の Python パーサは使わない。以前は rustpython で Python のソースを
//    Arrow の文へ変換する変換器があり、ここはその取りこぼしを補う側だった（変換器は
//    pandas_import_plan.md の 3-3 で消した。Python のコードは CPython が走らせる）。
//
// ⚠ CLI もエディタも Python のモジュールの型は**この抽出器だけ**で取る（`py_modules.rs`）ので、取れる型は同じ
//    （`compare_wasm_frontend.ps1` が守る不変条件）。


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
///
/// ⚠ テストだけが使う（1 つのファイルだけの型）。`import[py]` の型スタブは、たどって宣言を足してから `Any` にする
///   （[`extract_py_type_stubs_raw`]・`py_modules.rs` の `link_py_types`・pandas_import_plan.md の 1-3）。
#[cfg(test)]
pub(crate) fn extract_py_type_stubs(source: &str) -> Vec<crate::ast::Stmt> {
    let mut stmts = extract_py_type_stubs_raw(source);
    py_unknown_types_to_any(&mut stmts);
    stmts
}

/// [`extract_py_type_stubs`] の、このファイルが宣言していない型の名前を `Any` にする**前**の宣言（pandas_import_plan.md の 1-3）。
///
/// `import[py]` の型スタブは、再エクスポートと注釈のよそのクラスをたどって宣言を足してから `Any` にする
/// （`py_modules.rs` の `link_py_types`）。先に `Any` にすると、よそのモジュールのクラスの名前が消える。
pub(crate) fn extract_py_type_stubs_raw(source: &str) -> Vec<crate::ast::Stmt> {
    let lines: Vec<&str> = source.lines().collect();
    let mut stmts: Vec<crate::ast::Stmt> = Vec::new();
    let mut i = 0usize;
    // 直前にトップレベルのデコレータ（`@dataclass` など）があったか
    let mut decorated = false;
    // 直前のデコレータに `@overload` があったか（pandas_import_plan.md の 1-3）。
    // ⚠ `@overload` の宣言は型の検査器向けの別の形で、実行時に呼ばれるのは本体（`@overload` の無い同じ名前の `def`）だけ。
    //   各形は、その名前の本体の宣言の**本体の中**に入れ子で持たせる（[`PY_OVERLOAD_MARK`]）。型検査は `m.f(..)` の
    //   呼び出しで、CPython の型の検査器と同じく最初に当てはまる形を選ぶ（`read_csv(path)` は `DataFrame`）。
    //   モジュールの宣言として並べないのは、同じ名前の関数を増やすと Arrow の多重定義の解決（直接の呼び出し）に乗るため。
    let mut overloaded = false;
    // 本体を待っている `@overload` の各形（名前ごと・書いた順）
    let mut variants: Vec<(String, Vec<crate::ast::Stmt>)> = Vec::new();

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
            let deco = trimmed[1..].split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or("");
            if deco == "overload" || deco.ends_with(".overload") {
                overloaded = true;
            }
            i += 1;
            continue;
        }

        // トップレベルの class 定義（7-27）
        // ⚠ スタブ（`.ars`）を読んだ C# / JS のクラスは**通常の Arrow 宣言**として別経路で登録されるので、ここを通らない。
        if py_parse_class_name(trimmed).is_some() {
            let (decls, next) = py_class_stub(&lines, i, decorated);
            stmts.extend(decls);
            decorated = false;
            overloaded = false;
            i = next;
            continue;
        }

        // 関数定義（トップレベルのみ対象）
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            decorated = false;
            let is_variant = std::mem::replace(&mut overloaded, false);
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
                    let mut f = py_function(&name, params, arrow_ret);
                    if is_variant {
                        // `@overload` の形: 印と、`Literal[..]` の引数の値を付けて、本体を待つ
                        py_mark_overload(&mut f, &params_text);
                        match variants.iter_mut().find(|(n, _)| *n == name) {
                            Some((_, vs)) => vs.push(f),
                            None => variants.push((name.clone(), vec![f])),
                        }
                    } else {
                        // 本体: 待っていた形を本体の中に入れる
                        if let Some(k) = variants.iter().position(|(n, _)| *n == name) {
                            let (_, vs) = variants.remove(k);
                            if let crate::ast::Stmt::FnDef { body, .. } = &mut f {
                                *body = vs;
                            }
                        }
                        stmts.push(f);
                    }
                }
                i = py_block_end(&lines, body_line, func_indent);
                continue;
            }
        }

        // 注釈のあるモジュールの大域変数（`version: str` / `maxsize: int = ...`・pandas_import_plan.md の 1-4）。
        // ⚠ 注釈の無い `x = ..` は拾わない（型が分からない。名前の無いメンバーと同じく `Any`）
        if let Some((name, ty)) = py_module_global(py_strip_comment(trimmed).trim()) {
            stmts.push(crate::ast::Stmt::Let(name, Some(ty), crate::ast::Expr::None, crate::token::Span::unknown()));
        }

        decorated = false;
        overloaded = false;
        i += 1;
    }

    // 本体の無い `@overload`（`.pyi` は形だけを書く）: 最後の形を本体の代わりにし、戻り値はすべての形の戻り値を合わせたもの
    for (_, vs) in variants {
        let Some(crate::ast::Stmt::FnDef { name, params, .. }) = vs.last().cloned() else { continue };
        let rets: Vec<String> = vs
            .iter()
            .filter_map(|v| if let crate::ast::Stmt::FnDef { return_type, .. } = v { return_type.clone() } else { None })
            .collect();
        let mut f = py_function(&name, params, union_of(rets));
        if let crate::ast::Stmt::FnDef { body, .. } = &mut f {
            *body = vs;
        }
        stmts.push(f);
    }

    stmts
}

/// `@overload` の形に付ける印（本体の宣言の本体の中の入れ子の宣言のデコレータ）。型検査が呼び出しで形を選ぶ（1-3）。
pub(crate) const PY_OVERLOAD_MARK: &str = "py-overload";

/// `@overload` の形（関数・メソッドの本体の中に入れ子で持つ）か。
pub(crate) fn is_py_overload_variant(st: &crate::ast::Stmt) -> bool {
    matches!(st, crate::ast::Stmt::FnDef { decorators, .. }
        if decorators.iter().any(|d| matches!(d, crate::ast::Expr::Str(s) if &**s == PY_OVERLOAD_MARK)))
}

/// 宣言 `f` に `@overload` の形の印と、`Literal[..]` の引数の値を付ける（`params_text` は引数の並びの字句）。
fn py_mark_overload(f: &mut crate::ast::Stmt, params_text: &str) {
    if let crate::ast::Stmt::FnDef { decorators, .. } = f {
        decorators.push(crate::ast::Expr::Str(PY_OVERLOAD_MARK.into()));
        for (pname, value) in py_literal_params(params_text) {
            decorators.push(crate::ast::Expr::Str(format!("{PY_LITERAL_MARK}{pname}={value}").into()));
        }
    }
}

/// `@overload` の形の `Literal[..]` の引数の印（`py-literal:iterator=True`）。値の 1 つだけの `Literal` だけを書く。
pub(crate) const PY_LITERAL_MARK: &str = "py-literal:";

/// 引数の並び（`def f(` と `)` の間）から、`Literal[X]`（値が 1 つ）と注釈した引数の（名前, 値の綴り）。
fn py_literal_params(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in split_top_level(text, ',') {
        let decl = split_top_level(part.trim(), '=').first().map(|d| d.trim().to_string()).unwrap_or_default();
        let Some((name, ann)) = decl.split_once(':') else { continue };
        let ann = ann.trim();
        let inner = ann
            .strip_prefix("Literal[")
            .or_else(|| ann.strip_prefix("typing.Literal["))
            .and_then(|r| r.strip_suffix(']'));
        if let Some(inner) = inner {
            let values = split_top_level(inner, ',');
            if values.len() == 1 {
                out.push((name.trim().to_string(), values[0].trim().to_string()));
            }
        }
    }
    out
}

/// トップレベルのクラスのうち、**このファイルに無い基底**を持つものの、その基底の名前（書いた綴り・`indexing.IndexingMixin` など）。
/// `py_class_stub` はこの基底のメンバーを知らないので、クラスを開いておく（メンバーを検査しない）。`import[py]` の型スタブは
/// これをたどって基底のメンバーを写す（`py_modules.rs` の `link_py_types`・pandas_import_plan.md の 1-3）。
pub(crate) fn py_class_outside_bases(source: &str) -> std::collections::HashMap<String, Vec<String>> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = std::collections::HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        if py_line_indent(line) != 0 {
            continue;
        }
        let header = py_strip_comment(line).trim();
        let Some(name) = py_parse_class_name(header) else { continue };
        let bases: Vec<String> = match (header.find('('), header.rfind(')')) {
            (Some(o), Some(c)) if o < c => split_top_level(&header[o + 1..c], ',')
                .into_iter()
                .filter(|b| !b.is_empty() && *b != "object" && !b.contains('='))
                .filter(|b| !matches!(py_find_class(&lines, b), Some(j) if j != i))
                .map(|b| b.to_string())
                .collect(),
            _ => Vec::new(),
        };
        if !bases.is_empty() {
            out.insert(name, bases);
        }
    }
    out
}

/// クラスの宣言 `class` に、ほかのファイルの基底 `base` のメンバーを写す（派生に同じ名前があれば派生が勝つ・`py_class_stub` が
/// 同じファイルの基底にしているのと同じ規則）。⚠ `__init__` は写さない（派生は必ず `__init__` を持つ。書いていなければ
/// `py_class_stub` が引数を問わないものを置いている）。
pub(crate) fn py_merge_base_members(class: &mut crate::ast::Stmt, base: &crate::ast::Stmt) {
    use crate::ast::Stmt;
    let (Stmt::ClassDef { body, .. }, Stmt::ClassDef { body: base_body, .. }) = (class, base) else { return };
    let member = |s: &Stmt| match s {
        Stmt::FnDef { name, .. } | Stmt::Field { name, .. } => Some(name.clone()),
        _ => None,
    };
    let have: std::collections::HashSet<String> = body.iter().filter_map(member).collect();
    let inherited: Vec<Stmt> = base_body
        .iter()
        .filter(|s| member(s).is_some_and(|n| n != "__init__" && !have.contains(&n)))
        .cloned()
        .collect();
    body.splice(0..0, inherited);
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
    /// 関数の注釈に `fix` を当てる。⚠ 本体の中の `@overload` の各形も同じ（以前は本体の宣言だけで、`np.argmax(a)` が形の
    /// `a: ArrayLike` と照合されて偽の誤りになった・pandas_import_plan.md の 6-2）。
    fn fix_fn(st: &mut Stmt, fix: &dyn Fn(&mut String)) {
        let Stmt::FnDef { params, return_type, body, .. } = st else { return };
        for p in params.iter_mut() {
            if let Some(t) = p.type_ann.as_mut() {
                fix(t);
            }
        }
        if let Some(t) = return_type.as_mut() {
            fix(t);
        }
        for v in body.iter_mut() {
            fix_fn(v, fix);
        }
    }
    for st in stmts.iter_mut() {
        match st {
            Stmt::FnDef { .. } => fix_fn(st, &fix),
            Stmt::ClassDef { name, body, .. } => {
                // `typing.Self` はこのクラス（基底から写したメンバーなら写した先・PEP 673）。⚠ 以前は `Self` のまま残り、
                //   `df.head()` の型が中身の無い `Self` になっていた（pandas_import_plan.md の 1-3）。
                let this = name.clone();
                let self_to_class = |t: &mut String| *t = py_replace_type_word(t, "Self", &this);
                for m in body.iter_mut() {
                    match m {
                        Stmt::FnDef { .. } => fix_fn(m, &|t: &mut String| {
                            self_to_class(t);
                            fix(t);
                        }),
                        Stmt::Field { type_ann, .. } => {
                            self_to_class(type_ann);
                            fix(type_ann)
                        }
                        _ => {}
                    }
                }
            }
            Stmt::Let(_, Some(t), _, _) => fix(t),
            _ => {}
        }
    }
}

/// 型の字句 `ty` の中の名前 `from`（語として一致するものだけ）を `to` にする。
fn py_replace_type_word(ty: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(ty.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        out.push_str(if word == from { to } else { word.as_str() });
        word.clear();
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

/// 型の字句の中の、知らない大文字始まりの名前を `Any` にする（`py_unknown_types_to_any`）。Arrow の型の構成子
/// （`Union` / `Option` / `Any` / `None` …）と `known` の名前は残す。
fn py_type_names_to_any(ty: &str, known: &std::collections::HashSet<String>) -> String {
    const ARROW_TYPE_NAMES: &[&str] = &["Any", "None", "Union", "Option", "Intersection", "Result", "Self", "Undefined"];
    let mut out = String::with_capacity(ty.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            // 組み込みの protocol（`Sequence` …・6-8）も Arrow の型の名前（`py_type_to_arrow` が `typing` のものだけ残す）
            let keep = !word.starts_with(|c: char| c.is_ascii_uppercase())
                || ARROW_TYPE_NAMES.contains(&word.as_str())
                || known.contains(word.as_str())
                || crate::type_check::builtins::is_builtin_protocol(word);
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
///
/// 戻り値の宣言は、入れ子のクラス（`Tree.Node` の名前・Python の `__qualname__`）を先に、このクラスを最後に並べる。
fn py_class_stub(lines: &[&str], start: usize, decorated: bool) -> (Vec<crate::ast::Stmt>, usize) {
    let name = py_parse_class_name(py_strip_comment(lines[start]).trim()).unwrap_or_default();
    py_class_stub_named(lines, start, decorated, name, &std::collections::HashMap::new())
}

/// 同じ本体の先に書いた入れ子のクラス（素の名前 → (`class` の行, `Outer.Inner`)）。Python のクラスの本体の中では、入れ子の
/// クラスを素の名前で引ける（`class Leaf(Node):` の `Node` は同じ本体の `Node`）。
type PySiblings = std::collections::HashMap<String, (usize, String)>;

/// [`py_class_stub`] の本体。`qualname` はこのクラスの名前（入れ子なら `Outer.Inner`）。
///
/// 入れ子のクラス（クラス本体の `class`・pandas_import_plan.md 3-1）は `Outer.Inner` の名前で外へ持ち上げ、外側のクラスには
/// その型の値の属性（`Inner: type[Outer.Inner]`）を置く（`m.Tree.Node(v)` / `t.Leaf(w)` が引ける・削除した変換器と同じ形）。
/// ⚠ 以前は入れ子のクラスを読み飛ばしていたので、`m.Tree.Node` が `has no member 'Node'` の静的な誤りだった。
fn py_class_stub_named(
    lines: &[&str],
    start: usize,
    decorated: bool,
    qualname: String,
    siblings: &PySiblings,
) -> (Vec<crate::ast::Stmt>, usize) {
    use crate::ast::{Accessibility, Expr, FieldKind, Stmt};
    let header = py_strip_comment(lines[start]).trim();
    let name = qualname;
    let end = py_block_end(lines, start + 1, py_line_indent(lines[start]));
    let bases = py_header_bases(header);
    // 自分と、同じファイルの基底（その基底も）のメンバー。派生で同じ名前があれば派生が勝つ
    let (PyClassMembers { mut fields, mut methods, nested, open_body }, base_open) =
        py_class_all_members(lines, start, 0, siblings);
    let open = decorated || base_open || open_body;
    // 入れ子のクラス: 先に持ち上げ、外側のクラスにはその型の値の属性を置く
    let mut hoisted: Vec<Stmt> = Vec::new();
    let mut inner_map: PySiblings = PySiblings::new();
    for (line, inner_decorated) in nested {
        let Some(inner) = py_parse_class_name(py_strip_comment(lines[line]).trim()) else { continue };
        let q = format!("{name}.{inner}");
        let (decls, _) = py_class_stub_named(lines, line, inner_decorated, q.clone(), &inner_map);
        hoisted.extend(decls);
        inner_map.insert(inner.clone(), (line, q.clone()));
        fields.retain(|(n, _)| *n != inner);
        fields.push((inner, format!("type[{q}]")));
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
    hoisted.push(Stmt::ClassDef {
        src: None,
        name,
        template_params: vec![],
        // 例外のクラスは、継承をたどって行き着く組み込みの例外を基底に持たせる（`except m.Err:` の型検査・
        // pandas_import_plan.md の 2-1）。同じファイルの基底のクラスも書く（派生を基底の型の引数に渡せる・
        // `fn f(mut a: z.Animal)` に `z.Puppy()`・3-1。メンバーは上で写した）。ほかのファイルの基底は `link_py_types` が
        // 修飾名で足す（`py_merge_outside_bases`）
        bases: py_exception_base(lines, &bases, 0)
            .into_iter()
            .chain(py_same_file_bases(lines, start, &bases, siblings))
            .collect(),
        base_args: vec![],
        decorators,
        body,
    });
    (hoisted, end)
}

/// `class` の行に書いた基底（`object` とキーワード引数 `metaclass=..` を除く・書いた綴り）。
fn py_header_bases(header: &str) -> Vec<String> {
    match (header.find('('), header.rfind(')')) {
        (Some(o), Some(c)) if o < c => split_top_level(&header[o + 1..c], ',')
            .into_iter()
            .filter(|b| !b.is_empty() && *b != "object" && !b.contains('='))
            .map(|b| b.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

/// 基底の名前 `base` が指す同じファイルのクラスの `class` の行と、型の名前（入れ子の兄弟なら `Outer.Inner`・最上位なら素の名前）。
fn py_find_base(lines: &[&str], start: usize, base: &str, siblings: &PySiblings) -> Option<(usize, String)> {
    if let Some((line, q)) = siblings.get(base) {
        return Some((*line, q.clone()));
    }
    match py_find_class(lines, base) {
        Some(j) if j != start => Some((j, base.to_string())),
        _ => None,
    }
}

/// `class` の行 `start` のクラスのメンバーに、同じファイルの基底のメンバーを（基底の基底もたどって）足したもの。2 つ目は、
/// 同じファイルに無い基底（ライブラリのクラスなど）をどこかで継承しているか（メンバーの顔ぶれが分からない）。
///
/// 名前が重なれば派生が、基底どうしでは先に書いた基底が勝つ（CPython の MRO の近似）。`__init__` も受け継ぐ（Python と同じく、
/// 派生が `__init__` を書かなければ基底のものが使われる）。
/// ⚠ 以前は直接の基底のメンバーだけを写していたので、`class C(B)`・`class B(A)` の `C` に `A` のメソッドが無かった
///   （`c.show()` が `'C' has no member 'show'`・`Puppy(Dog(Animal))` の `__init__` が引数なし。pandas_import_plan.md 3-1）。
fn py_class_all_members(lines: &[&str], start: usize, depth: u32, siblings: &PySiblings) -> (PyClassMembers, bool) {
    use crate::ast::Stmt;
    let header = py_strip_comment(lines[start]).trim();
    let end = py_block_end(lines, start + 1, py_line_indent(lines[start]));
    let mut out = py_class_members(lines, start, end);
    let mut open = false;
    let bases = py_header_bases(header);
    for base in &bases {
        let bare = base.split('[').next().unwrap_or(base).trim();
        match py_find_base(lines, start, bare, siblings) {
            // ⚠ 深さの上限は循環（壊れたソース）の保険
            Some((b, _)) if depth < 20 => {
                // ⚠ 基底の基底は最上位のクラスから引く（基底が入れ子のクラスでも・近似）
                let (bm, base_open) = py_class_all_members(lines, b, depth + 1, &PySiblings::new());
                open |= base_open;
                for f in bm.fields {
                    py_add_field(&mut out.fields, f.0, f.1);
                }
                for m in bm.methods {
                    let Stmt::FnDef { name, .. } = &m else { continue };
                    if !out.methods.iter().any(|x| matches!(x, Stmt::FnDef { name: n, .. } if n == name)) {
                        out.methods.push(m);
                    }
                }
                open |= bm.open_body;
            }
            _ => open = true,
        }
    }
    (out, open)
}

/// 基底 `bases`（同じファイルのクラスはその基底もたどる）が行き着く組み込みの例外（`ValueError` など・Arrow の
/// `BUILTIN_EXCEPTIONS` にあるもの）。例外のクラスでなければ `None`。ほかのファイルの基底は `link_py_types` がたどる。
fn py_exception_base(lines: &[&str], bases: &[String], depth: u32) -> Option<String> {
    if depth > 20 {
        return None;
    }
    for b in bases {
        let bare = b.split('[').next().unwrap_or(b).trim();
        let last = bare.rsplit('.').next().unwrap_or(bare);
        if crate::type_check::names::BUILTIN_EXCEPTIONS.iter().any(|(n, _)| *n == last) && (bare == last || bare == format!("builtins.{last}")) {
            return Some(last.to_string());
        }
        if let Some(j) = py_find_class(lines, bare) {
            let header = py_strip_comment(lines[j]).trim();
            let inner: Vec<String> = match (header.find('('), header.rfind(')')) {
                (Some(o), Some(c)) if o < c => split_top_level(&header[o + 1..c], ',')
                    .into_iter()
                    .filter(|x| !x.is_empty() && !x.contains('='))
                    .map(|x| x.to_string())
                    .collect(),
                _ => Vec::new(),
            };
            if let Some(found) = py_exception_base(lines, &inner, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

/// `bases`（`class` の行 `start` に書いた基底）のうち、同じファイルのクラス（型引数は外した名前）。
fn py_same_file_bases(lines: &[&str], start: usize, bases: &[String], siblings: &PySiblings) -> Vec<String> {
    bases
        .iter()
        .filter_map(|b| py_find_base(lines, start, b.split('[').next().unwrap_or(b).trim(), siblings).map(|(_, q)| q))
        .collect()
}

/// 組み込みの例外の名前（`ValueError` など・Arrow の `BUILTIN_EXCEPTIONS`）か。
pub(crate) fn is_builtin_exception_name(name: &str) -> bool {
    crate::type_check::names::BUILTIN_EXCEPTIONS.iter().any(|(n, _)| *n == name)
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
    /// 入れ子のクラスの `class` の行と、デコレータが付いていたか（`py_class_stub_named` が持ち上げる）
    nested: Vec<(usize, bool)>,
    /// 本体に宣言でない文（`if` / `for` / `try` …）があるか。その中で定義した名前は読めないので、メンバーの顔ぶれが分からない
    open_body: bool,
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
    let mut out = PyClassMembers { fields: Vec::new(), methods: Vec::new(), nested: Vec::new(), open_body: false };
    let body_indent = lines[start + 1..end]
        .iter()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .map(|l| py_line_indent(l))
        .unwrap_or(4);
    let mut decos: Vec<String> = Vec::new();
    // メソッドの（名前, 戻り値の注釈, 引数の並び）。`name = staticmethod(f)` の `f` を引く（下の `py_method_wrapper`）
    let mut defs: Vec<(String, Option<String>, String)> = Vec::new();
    // 本体を待っている `@overload` の各形（名前ごと・書いた順）
    let mut variants: Vec<(String, Vec<crate::ast::Stmt>)> = Vec::new();
    let mut i = start + 1;
    while i < end {
        let line = lines[i];
        let t = py_strip_comment(line).trim();
        if t.is_empty() || py_line_indent(line) != body_indent {
            i += 1;
            continue;
        }
        // `to_f = staticmethod(to_f)` / `parse = classmethod(parse)`（デコレータを使わない古い書き方）: 名前の付いたメソッドを
        // 静的メソッド・クラスメソッドとして足し直す。⚠ 以前は `to_f` が普通のメソッド（先頭の引数を `self` として外す）のままだった
        if let Some((alias, deco, target)) = py_method_wrapper(t) {
            if let Some((_, ret, params)) = defs.iter().find(|(n, _, _)| *n == target) {
                let ret_ty = Some(ret.as_deref().map(|r| py_type_to_arrow(py_unquote(r))).unwrap_or_else(|| "Any".to_string()));
                let params = py_params(params, deco == "classmethod").unwrap_or_else(py_open_params);
                out.methods.retain(|m| !matches!(m, crate::ast::Stmt::FnDef { name, .. } if *name == alias));
                out.fields.retain(|(n, _)| *n != alias);
                out.methods.push(py_method(&alias, params, ret_ty, true));
            }
            decos.clear();
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
                defs.push((mname.clone(), ret.clone(), params.clone()));
                let ret_ty = ret.as_deref().map(|r| py_type_to_arrow(py_unquote(r)));
                let is_static = decos.iter().any(|d| d == "staticmethod" || d == "classmethod");
                // `@classmethod` の先頭の引数（`cls`）は呼び出しで渡さない。`@staticmethod` は先頭も本物の引数。
                // ⚠ 以前は両方とも先頭を残していたので、`Box.of(v)` が `takes 2 argument(s)` の偽の誤りだった。
                let drops_first = !decos.iter().any(|d| d == "staticmethod");
                if decos.iter().any(|d| d == "property" || d == "functools.cached_property" || d == "cached_property") {
                    py_add_field(&mut out.fields, mname, ret_ty.unwrap_or_else(|| "Any".to_string()));
                } else if !decos.iter().any(|d| d.ends_with(".setter") || d.ends_with(".deleter")) {
                    let parsed = py_params(&params, drops_first).unwrap_or_else(py_open_params);
                    let ret_ty = if mname == "__init__" { Some("None".to_string()) } else { Some(ret_ty.unwrap_or_else(|| "Any".to_string())) };
                    let mut method = py_method(&mname, parsed, ret_ty, is_static);
                    if decos.iter().any(|d| d == "overload" || d.ends_with(".overload")) {
                        // `@overload` の形はモジュールの関数と同じく本体の中に入れ子で持つ（型検査が最初に当てはまる形を選ぶ・6-4）。
                        // ⚠ 以前は同じ名前の `def` を上書きしていたので、最後の形だけが残った
                        py_mark_overload(&mut method, &params);
                        match variants.iter_mut().find(|(n, _)| *n == mname) {
                            Some((_, vs)) => vs.push(method),
                            None => variants.push((mname.clone(), vec![method])),
                        }
                    } else {
                        if let Some(k) = variants.iter().position(|(n, _)| *n == mname) {
                            let (_, vs) = variants.remove(k);
                            if let crate::ast::Stmt::FnDef { body, .. } = &mut method {
                                *body = vs;
                            }
                        }
                        out.methods.retain(|m| !matches!(m, crate::ast::Stmt::FnDef { name, .. } if *name == mname));
                        out.methods.push(method);
                    }
                }
                decos.clear();
                i = body_end;
                continue;
            }
        }
        if t.starts_with("class ") {
            // 入れ子のクラス（`py_class_stub_named` が `Outer.Inner` として持ち上げる）
            out.nested.push((i, !decos.is_empty()));
            decos.clear();
            i = py_block_end(lines, i + 1, body_indent);
            continue;
        }
        // 宣言でない文（`if debug: level = 10`）: その中の名前は読まないので、クラスを開いておく（無い名前を誤りにしない）
        if ["if ", "for ", "while ", "try:", "with ", "match ", "async for ", "async with "].iter().any(|k| t.starts_with(k)) {
            out.open_body = true;
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
    // 本体の無い `@overload`（`.pyi` は形だけを書く）: モジュールの関数と同じく、最後の形を本体の代わりにする
    for (mname, vs) in variants {
        let Some(mut method) = vs.last().cloned() else { continue };
        let rets: Vec<String> = vs
            .iter()
            .filter_map(|v| if let crate::ast::Stmt::FnDef { return_type, .. } = v { return_type.clone() } else { None })
            .collect();
        if let crate::ast::Stmt::FnDef { return_type, decorators, body, .. } = &mut method {
            *return_type = Some(union_of(rets));
            decorators.clear();
            *body = vs;
        }
        out.methods.retain(|m| !matches!(m, crate::ast::Stmt::FnDef { name, .. } if *name == mname));
        out.methods.push(method);
    }
    out
}

/// クラス本体の 1 行が `name = staticmethod(f)` / `name = classmethod(f)` なら（`name`, `staticmethod` か `classmethod`, `f`）。
fn py_method_wrapper(t: &str) -> Option<(String, &'static str, String)> {
    let (lhs, rhs) = t.split_once('=')?;
    let alias = lhs.trim();
    let rhs = rhs.trim();
    if rhs.starts_with('=') || alias.is_empty() || !alias.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let (deco, rest) = [("staticmethod", "staticmethod("), ("classmethod", "classmethod(")]
        .into_iter()
        .find_map(|(d, p)| rhs.strip_prefix(p).map(|r| (d, r)))?;
    let target = rest.strip_suffix(')')?.trim();
    (!target.is_empty() && target.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then(|| (alias.to_string(), deco, target.to_string()))
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

/// トップレベルの 1 行が注釈のある大域変数（`name: T` / `name: T = …`）なら（名前, 型）。`_` で始まる名前は拾わない
/// （関数と同じ）。⚠ `else: x = 1` のような 1 行の複合文を大域変数と読まないよう、Python の予約語で始まる行は除く。
fn py_module_global(t: &str) -> Option<(String, String)> {
    const KEYWORDS: &[&str] = &[
        "else", "elif", "except", "finally", "try", "if", "for", "while", "with", "match", "case", "class", "def",
        "async", "lambda", "global", "nonlocal", "return", "yield", "await", "not", "pass", "del", "assert", "raise",
        "import", "from",
    ];
    let ident_end = t.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(t.len());
    let name = &t[..ident_end];
    if name.starts_with('_') || KEYWORDS.contains(&name) || !t[ident_end..].trim_start().starts_with(':') {
        return None;
    }
    py_attr_decl(t)
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
        // ⚠ `*args` の後の引数はキーワード専用だが、Arrow の仮引数には印が無く、位置引数が先に束縛される。`a.resize(5)`（`*new_shape,
        // refcheck: bool = ...`）の `5` を `bool` と照合して偽の誤りになったので、型を見ない（pandas_import_plan.md の 6-3）
        let ty = if varargs.is_some() {
            "Any".to_string()
        } else {
            ann.map(|a| py_param_type(&py_type_to_arrow(py_unquote(a)))).unwrap_or_else(|| "Any".to_string())
        };
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
    // ⚠ 以前は 40 行で打ち切っていたので、引数の多い関数（pandas の `read_csv` は約 60 行）が閉じ括弧に届かず、宣言ごと
    //   落ちていた（`pd.read_csv` が `Any`・pandas_import_plan.md の 1-3）。上限は壊れたソースで最後まで読み続けないための保険。
    let mut depth = 0i32;
    let mut combined = String::new();
    let mut body_line = start + 1;
    for (j, &line) in lines[start..].iter().enumerate().take(1000) {
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
            // 大域変数・クラスの属性の印（PEP 591 / PEP 526）: 型は中身
            ("Final" | "ClassVar", [t]) => t.clone(),
            ("Union", ts) if !ts.is_empty() => union_of(ts.to_vec()),
            // list / dict / set は写さずに CPython の物のまま届く（`py_to_tl`・pandas_import_plan.md の D-8・D-12・2-7）
            ("List" | "list", [t]) => format!("py_list[{t}]"),
            ("Set" | "set", [t]) => format!("py_set[{t}]"),
            ("Dict" | "dict", [k, v]) => format!("py_dict[{k}, {v}]"),
            // frozenset は不変なので、実行時は Arrow の set に写して届く（`py_to_tl`・2-6）
            ("FrozenSet" | "frozenset", [t]) => format!("set[{t}]"),
            // 可変長（`tuple[int, ...]`）と空（`tuple[()]`）は要素の並びを Arrow で書けないので素の `tuple`（形だけ照合）
            ("Tuple" | "tuple", _) if raw_args.iter().any(|t| matches!(t.trim(), "..." | "()")) => "tuple".to_string(),
            ("Tuple" | "tuple", ts) if !ts.is_empty() => format!("tuple[{}]", ts.join(", ")),
            // `typing` / `collections.abc` の `Sequence[int]` などは組み込みの protocol（構造で照合する・
            // pandas_import_plan.md の 6-8）。⚠ 型引数の個数が宣言と違えば検査しない（`Any`）
            (b, ts) if crate::type_check::builtins::protocol_arity(b).is_some_and(|n| n == ts.len() && n > 0) => {
                format!("{b}[{}]", ts.join(", "))
            }
            // 知らないジェネリクス（`Callable[...]` / 利用者のクラス `Box[int]` など）は検査しない
            _ => "Any".to_string(),
        };
    }
    match a {
        "str" | "int" | "float" | "bool" | "None" | "Any" | "bytes" | "tuple" => a.to_string(),
        // 要素の型を書かない list / dict / set（D-12・2-7）
        "list" | "List" => "py_list[Any]".to_string(),
        "dict" | "Dict" => "py_dict[Any, Any]".to_string(),
        "set" | "Set" => "py_set[Any]".to_string(),
        "frozenset" => "set".to_string(),
        // `typing.LiteralString`（PEP 675）は文字列（typeshed の `sys.platform: LiteralString`・pandas_import_plan.md の 1-6）
        "LiteralString" | "typing.LiteralString" | "typing_extensions.LiteralString" => "str".to_string(),
        // Python のバイト列はどれも Arrow の bytes として受け取る（ffi_value_exchange_plan.md 5-3）。要素が 1 バイトでない
        // memoryview は Python のオブジェクトのまま届き、境界の照合で FfiTypeError になる
        "bytearray" | "memoryview" => "bytes".to_string(),
        // 型引数を書かない組み込みの protocol（`Sequence` / `typing.Sized` / `collections.abc.Iterable`・6-8）。型引数を取るものは
        // `Any` で埋める（Python の素の `Sequence` は `Sequence[Any]`）
        _ if py_builtin_protocol_name(a).is_some() => {
            let name = py_builtin_protocol_name(a).expect("checked");
            match crate::type_check::builtins::protocol_arity(name).unwrap_or(0) {
                0 => name.to_string(),
                n => format!("{name}[{}]", vec!["Any"; n].join(", ")),
            }
        }
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

/// 注釈の名前 `a`（`Sequence` / `typing.Sized` / `collections.abc.Iterable` / `abc.Mapping`）が組み込みの protocol なら、その名前
/// （pandas_import_plan.md の 6-8）。修飾は `typing` / `typing_extensions` / `collections.abc` / `abc` だけ（ほかのモジュールの
/// 同じ名前のクラスは違う型）。
fn py_builtin_protocol_name(a: &str) -> Option<&str> {
    let (prefix, name) = match a.rsplit_once('.') {
        Some((p, n)) => (Some(p), n),
        None => (None, a),
    };
    if !matches!(prefix, None | Some("typing" | "typing_extensions" | "collections.abc" | "abc")) {
        return None;
    }
    crate::type_check::builtins::is_builtin_protocol(name).then_some(name)
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
        assert_eq!(types, vec!["Any", "Box", "Option[py_list[Any]]"]);
        assert_eq!(return_type.as_deref(), Some("Box"));
    }

    /// Python のバイト列の注釈はどれも `bytes`（ffi_value_exchange_plan.md 5-3）。
    #[test]
    fn byte_sequences_are_bytes() {
        assert_eq!(py_type_to_arrow("bytes"), "bytes");
        assert_eq!(py_type_to_arrow("bytearray"), "bytes");
        assert_eq!(py_type_to_arrow("Optional[memoryview]"), "Option[bytes]");
        assert_eq!(py_type_to_arrow("list[bytearray]"), "py_list[bytes]");
    }

    /// 要素型を落とさない（7-8）。以前は `dict[K, V]` → `dict`・`set[tuple[int, int]]` → `set[tuple]`・`frozenset[T]` → `Any`・
    /// `Union[List[int], None]` は中を変換しないまま・`list[int] | None` → `Any` だった。
    #[test]
    fn element_types_are_kept() {
        assert_eq!(py_type_to_arrow("set[tuple[int, int]]"), "py_set[tuple[int, int]]");
        assert_eq!(py_type_to_arrow("frozenset[str]"), "set[str]");
        assert_eq!(py_type_to_arrow("FrozenSet[int]"), "set[int]");
        assert_eq!(py_type_to_arrow("dict[str, list[int]]"), "py_dict[str, py_list[int]]");
        assert_eq!(py_type_to_arrow("Dict[str, Tuple[int, float]]"), "py_dict[str, tuple[int, float]]");
        assert_eq!(py_type_to_arrow("Union[List[int], None]"), "Option[py_list[int]]");
        assert_eq!(py_type_to_arrow("Union[int, str, None]"), "Union[int, str, None]");
        assert_eq!(py_type_to_arrow("list[int] | None"), "Option[py_list[int]]");
        assert_eq!(py_type_to_arrow("int | str"), "Union[int, str]");
        assert_eq!(py_type_to_arrow("typing.Optional[typing.List[str]]"), "Option[py_list[str]]");
        // Arrow が表せないものは検査しない側
        assert_eq!(py_type_to_arrow("tuple[int, ...]"), "tuple");
        // `Sequence[int]` は組み込みの protocol（6-8・`typing_names_are_builtin_protocols`）。以前は `Any` だった
        assert_eq!(py_type_to_arrow("Sequence[int]"), "Sequence[int]");
        assert_eq!(py_type_to_arrow("Callable[[int], str]"), "Any");
        assert_eq!(py_type_to_arrow("Optional[Any]"), "Any");
        assert_eq!(py_type_to_arrow("Person"), "Person");
        assert_eq!(py_type_to_arrow("Final[int]"), "int");
    }

    /// 例外のクラスは、継承をたどって行き着く組み込みの例外を基底に持つ（`except m.Err:` の型検査・pandas_import_plan.md の 2-1）。
    /// 同じファイルの基底のクラスも基底に持つ（派生を基底の型で受ける・3-1）。ほかのファイルの基底は持たない（`link_py_types` が
    /// 修飾名で足す）。
    #[test]
    fn exception_classes_keep_their_builtin_base() {
        use crate::ast::Stmt;
        let src = "class A(ValueError):\n    pass\nclass B(A):\n    pass\nclass C(builtins.KeyError):\n    pass\nclass D:\n    pass\n\
                   class E(D):\n    pass\nclass F(other.Base):\n    pass\n";
        let bases: Vec<(String, Vec<String>)> = extract_py_type_stubs(src)
            .into_iter()
            .filter_map(|s| if let Stmt::ClassDef { name, bases, .. } = s { Some((name, bases)) } else { None })
            .collect();
        let of = |n: &str| bases.iter().find(|(c, _)| c == n).map(|(_, b)| b.clone()).unwrap();
        assert_eq!(of("A"), vec!["ValueError".to_string()]);
        assert_eq!(of("B"), vec!["ValueError".to_string(), "A".to_string()]);
        assert_eq!(of("C"), vec!["KeyError".to_string()]);
        assert!(of("D").is_empty() && of("F").is_empty());
        assert_eq!(of("E"), vec!["D".to_string()]);
    }

    /// `@classmethod` の `cls` は呼び出しで渡さない・`@staticmethod` の先頭は本物の引数・`f = staticmethod(f)` も同じ・
    /// 基底の基底のメンバーも持つ（pandas_import_plan.md 3-1）。
    #[test]
    fn class_and_static_methods_and_deep_bases() {
        let src = "class A:
    def __init__(self, v):
        self.v = v
    def show(self) -> str:
        return ''
class B(A):
    @classmethod
    def of(cls, v) -> 'B':
        return cls(v)
    @staticmethod
    def twice(x):
        return x
    def to_f(c):
        return c
    to_f = staticmethod(to_f)
    def parse(cls, t):
        return cls(t)
    parse = classmethod(parse)
class C(B):
    pass
";
        let stmts = extract_py_type_stubs(src);
        let (fields, methods, open) = class_parts(&stmts, "C");
        let m = |n: &str| methods.iter().find(|(name, _, _)| name == n).cloned().unwrap_or_else(|| panic!("no method {n} in {methods:?}"));
        assert_eq!(m("of").1, vec!["v:Any".to_string()], "classmethod: no cls");
        assert_eq!(m("twice").1, vec!["x:Any".to_string()], "staticmethod: the first parameter is real");
        assert_eq!(m("to_f").1, vec!["c:Any".to_string()], "`to_f = staticmethod(to_f)`");
        assert_eq!(m("parse").1, vec!["t:Any".to_string()], "`parse = classmethod(parse)`");
        assert_eq!(m("show").2, "str", "a member of the base of the base");
        assert_eq!(m("__init__").1, vec!["self:".to_string(), "v:Any".to_string()], "`__init__` of the base of the base");
        assert!(!fields.iter().any(|(n, _)| n == "to_f" || n == "parse"), "the wrappers are not attributes: {fields:?}");
        assert!(!open);
    }

    /// クラスのメソッドの `@overload` は、モジュールの関数と同じく本体の宣言の中に入れ子で持つ（pandas_import_plan.md の 6-4）。
    /// 本体の無い形（`.pyi`）は最後の形が本体の代わりで、戻り値はすべての形を合わせたもの。形の注釈の知らない名前は `Any`（6-2）。
    #[test]
    fn class_method_overloads_are_nested() {
        use crate::ast::Stmt;
        let src = "class C:
    @overload
    def get(self, key: ArrayLike) -> int: ...
    @overload
    def get(self, key: str, default: str) -> str: ...
    def get(self, key, default=None):
        return key
    @overload
    def only(self, a: int) -> int: ...
    @overload
    def only(self, a: str) -> str: ...
";
        let stmts = extract_py_type_stubs(src);
        let Some(Stmt::ClassDef { body, .. }) = stmts.iter().find(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "C")) else {
            panic!("{stmts:?}")
        };
        let method = |wanted: &str| {
            let found: Vec<&Stmt> =
                body.iter().filter(|m| matches!(m, Stmt::FnDef { name, .. } if name == wanted)).collect();
            assert_eq!(found.len(), 1, "{wanted}: {body:?}");
            found[0]
        };
        let Stmt::FnDef { body: variants, return_type, .. } = method("get") else { panic!() };
        assert_eq!(variants.len(), 2);
        assert_eq!(return_type.as_deref(), Some("Any"), "the implementation has no annotation");
        assert!(variants.iter().all(is_py_overload_variant));
        let Stmt::FnDef { params, .. } = &variants[0] else { panic!() };
        assert_eq!(params[1].type_ann.as_deref(), Some("Any"), "an unknown name in a variant");
        let Stmt::FnDef { body: variants, return_type, .. } = method("only") else { panic!() };
        assert_eq!(variants.len(), 2);
        assert_eq!(return_type.as_deref(), Some("Union[int, str]"));
    }

    /// `*args` の後のキーワード専用引数は型を見ない。Arrow の仮引数には印が無く、位置引数が先に束縛される（6-3）。
    #[test]
    fn keyword_only_after_varargs_is_any() {
        let params = py_params("*shape: int, refcheck: bool = ...", false).expect("params");
        let refcheck = params.iter().find(|p| p.name == "refcheck").expect("refcheck");
        assert_eq!(refcheck.type_ann.as_deref(), Some("Any"));
        let params = py_params("a: int, *, flag: bool = ...", false).expect("params");
        let flag = params.iter().find(|p| p.name == "flag").expect("flag");
        assert_eq!(flag.type_ann.as_deref(), Some("bool"), "a bare `*` takes no positional argument anyway");
    }

    /// `typing` / `collections.abc` の `Sequence` などは組み込みの protocol として読む（pandas_import_plan.md の 6-8）。
    /// 型引数を書かなければ `Any` で埋める。ほかのモジュールで修飾した同じ名前・個数の違う型引数は検査しない（`Any`）。
    #[test]
    fn typing_names_are_builtin_protocols() {
        assert_eq!(py_type_to_arrow("Sequence[int]"), "Sequence[int]");
        assert_eq!(py_type_to_arrow("typing.Iterable[str]"), "Iterable[str]");
        assert_eq!(py_type_to_arrow("Mapping[str, int]"), "Mapping[str, int]");
        assert_eq!(py_type_to_arrow("Sequence"), "Sequence[Any]");
        assert_eq!(py_type_to_arrow("collections.abc.Mapping"), "Mapping[Any, Any]");
        assert_eq!(py_type_to_arrow("SupportsIndex"), "SupportsIndex");
        assert_eq!(py_type_to_arrow("Iterable[int] | None"), "Option[Iterable[int]]");
        assert_eq!(py_type_to_arrow("Mapping[str]"), "Any", "wrong number of type arguments");
        assert_eq!(py_type_to_arrow("Sized[int]"), "Any", "Sized takes no type arguments");
        assert_eq!(py_type_to_arrow("np.Sized"), "Any", "another module's Sized is not the built-in protocol");
    }

    /// 入れ子のクラスは `Outer.Inner` の名前で持ち上げ、外側のクラスにはその型の値の属性を置く。入れ子のクラスの基底は同じ本体の
    /// 兄弟を先に引く。本体に宣言でない文（`if`）があるクラスは開いておく（pandas_import_plan.md 3-1）。
    #[test]
    fn nested_classes_are_hoisted() {
        use crate::ast::Stmt;
        let src = "class Node:
    pass
class Tree:
    class Node:
        def __init__(self, value):
            self.value = value
    class Leaf(Node):
        pass
class Settings:
    debug = False
    if debug:
        level = 10
";
        let stmts = extract_py_type_stubs(src);
        let names: Vec<&str> = stmts.iter().filter_map(|s| if let Stmt::ClassDef { name, .. } = s { Some(name.as_str()) } else { None }).collect();
        assert_eq!(names, vec!["Node", "Tree.Node", "Tree.Leaf", "Tree", "Settings"]);
        let (fields, _, _) = class_parts(&stmts, "Tree");
        assert_eq!(fields, vec![("Node".to_string(), "type[Tree.Node]".to_string()), ("Leaf".to_string(), "type[Tree.Leaf]".to_string())]);
        let (_, methods, _) = class_parts(&stmts, "Tree.Leaf");
        assert_eq!(methods[0].1, vec!["self:".to_string(), "value:Any".to_string()], "the sibling `Node`, not the top-level one");
        let Some(Stmt::ClassDef { bases, .. }) = stmts.iter().find(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "Tree.Leaf")) else {
            panic!()
        };
        assert_eq!(bases, &vec!["Tree.Node".to_string()]);
        let (_, _, open) = class_parts(&stmts, "Settings");
        assert!(open, "a class body with an `if`: the members are open");
    }

    /// 注釈のあるモジュールの大域変数は型つきの `let` の宣言になる（pandas_import_plan.md の 1-4）。注釈の無いもの・`_` で
    /// 始まるもの・1 行の複合文（`else: x = 1`）は拾わない。
    #[test]
    fn annotated_module_globals_are_declared() {
        use crate::ast::Stmt;
        let src = "version: str\nmaxsize: int = 9\nlimit: Final[float] = 1.0\nplain = 3\n_hidden: int = 1\n\
                   if x:\n    pass\nelse: y = 1\nclass C:\n    n: int\n";
        let globals: Vec<(String, Option<String>)> = extract_py_type_stubs(src)
            .into_iter()
            .filter_map(|s| if let Stmt::Let(n, t, _, _) = s { Some((n, t)) } else { None })
            .collect();
        assert_eq!(
            globals,
            vec![
                ("version".to_string(), Some("str".to_string())),
                ("maxsize".to_string(), Some("int".to_string())),
                ("limit".to_string(), Some("float".to_string())),
            ]
        );
    }
}

