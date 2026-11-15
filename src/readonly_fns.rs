// readonly_fns.rs — 「読むだけの関数」の解析（copy_alloc_speed_plan.md 4-1・4-2・D-11）。
//
// `mut` の値を `let self` / `let` 仮引数で受けるときの複製は、呼ばれた側が**何も書き換えず、受けた値の中身を
// 外へ出さない**なら結果を変えない（速度の調査で、時間の大半がこの複製だった）。ここは、そう言い切れる関数の
// **名前**の集合を、プログラム全体の AST（取り込んだモジュールの本体を含む）から求める。
//
// 判定は**名前で閉じる**（型の注釈に頼らない・D-11 の 3）: 名前 `f` が集合に入るのは、プログラム中の `f` という名前の
// 定義（最上位の関数・メソッド・入れ子の関数・どのモジュールでも）が**全部**下の条件を満たすときだけ。
// 実行時は、呼ばれた関数の名前がこの集合にあるかを見ればよい（どのクラスの `f` が呼ばれても読むだけ）。
//
// 関数が「読むだけ」である条件（1 つでも外れたら入れない。**分からないものは入れない**）:
//   1. `fn`（`gen`・`meta`・抽象・本体の無い宣言・デコレータつきは除く）
//   2. `mut` の仮引数（`mut self` を含む）が無い — 書き換えうる別の経路を持たない（`a.peek(a)` の形を除く）
//   3. 戻り値の型が `int` / `float` / `bool` / `str` / `None` — 受けた値の中身をそのまま返さない
//   4. 本体の文が、宣言・ローカルへの代入・制御構文・`return` / `raise` / `break` / `continue` だけ
//      （属性・添字への代入、`static`、`freeze`、`yield`、入れ子の定義、非同期の投入、イベントの購読は除く）
//   5. 本体の呼び出しが、読むだけの名前・書き換えない組み込み関数・書き換えない組み込み型のメソッドだけ
//      （関数の値の呼び出し・クラスの生成・ローカルに隠された名前・知らない名前は除く）
//   6. `===`（同一性の比較）を使わない — 受けた値が写しかどうかを見分けられる
// 演算子・添字・`for`・`str()` などが呼ぶ特殊メソッド（`__add__`・`__eq__`・`__str__`…）は名前に現れないので、
// **利用者が定義した特殊メソッド（`__init__` を除く）が全部読むだけ**のときだけ、どの関数も読むだけとみなす
// （1 つでも外れれば集合は空）。
//
// ⚠ 呼ばれている間に**別のスレッド**（非同期タスク）が同じ値を書き換える場合は、この解析では防げない。
//   使う側が、走っているタスクが無いときだけ複製を省く（同 D-7・D-8）。
// ⚠ コンパイル済みモジュール（`.arc`）の関数は、取り込んだ側の AST には宣言（本体の無いスタブ）しか無いので
//   入らない（同 4-5 で要約を `.arc` に載せる）。

use std::collections::{HashMap, HashSet};

use crate::ast::{CallArg, DictEntry, Expr, Param, SeqEntry, Stmt};

/// 読むだけの関数の名前の集合。
#[derive(Debug, Default, Clone)]
pub struct ReadonlyFns {
    names: HashSet<String>,
    /// **自分の `mut` 仮引数だけを書き換える関数**の名前（copy_alloc_speed_plan.md 4-6）。読むだけの条件のうち
    /// 「`mut` の仮引数が無い」と「属性・添字へ代入しない」を、「代入の先は自分の `mut` 仮引数（`mut self` を含む）を
    /// 根にした経路だけ」に緩めたもの。こういう関数の `let` 仮引数へは、渡す値が `mut` の引数と**重ならない**と
    /// 呼ぶ側が言えるとき（同じ変数の別のフィールド）、複製せずに渡せる。
    confined: HashSet<String>,
    /// 解析した定義の数（診断用）。
    pub defs_total: usize,
    /// そのうち、読むだけと判定した名前に属する定義の数（診断用）。
    pub defs_readonly: usize,
}

impl ReadonlyFns {
    /// `name` という関数・メソッドが読むだけか（載っていない名前は `false` ＝ 複製する側）。
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// `name` という関数・メソッドが、自分の `mut` 仮引数だけを書き換えるか（読むだけの関数も含む）。
    pub fn is_confined_writer(&self, name: &str) -> bool {
        self.names.contains(name) || self.confined.contains(name)
    }

    /// 自分の `mut` 仮引数だけを書き換える関数の名前の一覧（並べ替え済み・読むだけの関数は含まない）。
    pub fn sorted_confined(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.confined.iter().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// 名前の一覧（並べ替え済み・診断とテスト用）。
    pub fn sorted_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.names.iter().map(String::as_str).collect();
        v.sort_unstable();
        v
    }
}

/// 関数の定義 1 つ（解析の単位）。
struct Def<'a> {
    name: &'a str,
    params: &'a [Param],
    return_type: Option<&'a str>,
    body: &'a [Stmt],
    /// `fn` 以外（`gen`・`meta`）・抽象・デコレータつきなど、最初から読むだけにしない定義。
    excluded: bool,
}

/// 書き換えない組み込み関数（引数の値を書き換えず、利用者のコードを特殊メソッド経由でしか呼ばない）。
/// ⚠ ここに無い組み込み関数の呼び出しは「読むだけでない」とみなす（`setattr`・`open`・`next`・`input` など）。
const READONLY_BUILTIN_FNS: &[&str] = &[
    "abs", "all", "any", "bool", "chr", "divmod", "enumerate", "float", "getenv", "hash", "int", "isinstance",
    "len", "max", "min", "ord", "pow", "print", "range", "repr", "round", "sorted", "str", "sum", "type", "zip",
];

/// 戻り値の型として認めるもの（受けた値の中身を指さない）。
const SCALAR_RETURN_TYPES: &[&str] = &["int", "float", "bool", "str", "None"];

/// プログラム全体から、読むだけの関数の名前を求める。
pub fn analyze(stmts: &[Stmt]) -> ReadonlyFns {
    let mut defs: Vec<Def<'_>> = Vec::new();
    // 本体を集められなかった定義の名前（制御構文の中の定義）。読むだけにしない
    let mut blocked: HashSet<String> = HashSet::new();
    collect_defs(stmts, &mut defs, &mut blocked);

    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, d) in defs.iter().enumerate() {
        by_name.entry(d.name).or_default().push(i);
    }
    // 利用者が定義した特殊メソッド（`__init__` は生成のときだけ走る。生成は条件 5 で除いている）
    let dunders: Vec<&str> = by_name
        .keys()
        .copied()
        .filter(|n| n.starts_with("__") && n.ends_with("__") && *n != "__init__")
        .collect();
    // `for` が呼ぶ `__iter__` / `__next__` は `gen` で書くことが多いので、全体の条件から外し、
    // 定義があるプログラムでは `for` を含む関数を読むだけにしない（下の `Ctx::user_iter`）。
    let user_iter = by_name.contains_key("__iter__") || by_name.contains_key("__next__");

    // 最大の集合から始めて、条件を満たさない名前を外していく（不動点。再帰・相互再帰は「読むだけ」と仮定して始まる）
    let mut names: HashSet<&str> = by_name
        .iter()
        .filter(|(n, idxs)| !blocked.contains(**n) && idxs.iter().all(|&i| signature_ok(&defs[i])))
        .map(|(n, _)| *n)
        .collect();
    loop {
        let before = names.len();
        let snapshot = names.clone();
        names.retain(|n| {
            by_name[n].iter().all(|&i| {
                let d = &defs[i];
                let mut ctx = Ctx { readonly: &snapshot, locals: HashSet::new(), user_iter, writable: HashSet::new() };
                ctx.declare_params(d.params);
                collect_locals(d.body, &mut ctx.locals);
                ctx.stmts_ok(d.body)
            })
        });
        if names.len() == before {
            break;
        }
    }
    // 特殊メソッドが 1 つでも読むだけでなければ、演算子・添字・`str()` を使うどの関数も言い切れない
    let dunders_ok = dunders.iter().all(|n| *n == "__iter__" || *n == "__next__" || names.contains(n));
    if !dunders_ok {
        names.clear();
    }
    // 自分の `mut` 仮引数だけを書き換える関数（4-6）。読むだけの集合が決まってから求める（呼び出しは読むだけの名前だけ）。
    // ⚠ 特殊メソッドの条件が外れていれば（`names` が空）、こちらも空にする（演算子が何を呼ぶか分からない）。
    let confined: HashSet<&str> = if dunders_ok {
        by_name
            .iter()
            .filter(|(n, idxs)| {
                !blocked.contains(**n)
                    && !names.contains(**n)
                    && idxs.iter().all(|&i| {
                        let d = &defs[i];
                        if !confined_signature_ok(d) {
                            return false;
                        }
                        let writable = d.params.iter().filter(|p| p.mutable).map(|p| p.name.clone()).collect();
                        let mut ctx = Ctx { readonly: &names, locals: HashSet::new(), user_iter, writable };
                        ctx.declare_params(d.params);
                        collect_locals(d.body, &mut ctx.locals);
                        ctx.stmts_ok(d.body)
                    })
            })
            .map(|(n, _)| *n)
            .collect()
    } else {
        HashSet::new()
    };
    let defs_readonly = defs.iter().filter(|d| names.contains(d.name)).count();
    ReadonlyFns {
        names: names.into_iter().map(str::to_string).collect(),
        confined: confined.into_iter().map(str::to_string).collect(),
        defs_total: defs.len(),
        defs_readonly,
    }
}

/// 4-6 の関数の条件 1〜3: `mut` の仮引数はあってよい（それだけを書き換える）。ほかは読むだけと同じ。
fn confined_signature_ok(d: &Def<'_>) -> bool {
    !d.excluded
        && d.params.iter().all(|p| !p.variadic)
        && d.return_type.is_some_and(|t| SCALAR_RETURN_TYPES.contains(&t))
}

/// 条件 1〜3（本体を見ずに決まるもの）。
fn signature_ok(d: &Def<'_>) -> bool {
    !d.excluded
        && d.params.iter().all(|p| !p.mutable && !p.variadic)
        && d.return_type.is_some_and(|t| SCALAR_RETURN_TYPES.contains(&t))
}

/// プログラム中の関数の定義を全部集める（クラス・trait・取り込んだモジュール・関数の本体の中の入れ子も）。
fn collect_defs<'a>(stmts: &'a [Stmt], out: &mut Vec<Def<'a>>, blocked: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::FnDef { name, params, return_type, body, is_abstract, decorators, .. } => {
                // 本体の無い宣言（`.ars` のスタブ・trait の抽象メソッド）は中身が分からない
                let no_body = body.iter().all(|b| matches!(b, Stmt::Pass)) || is_stub_body(body);
                out.push(Def {
                    name,
                    params,
                    return_type: return_type.as_deref(),
                    body,
                    excluded: *is_abstract || no_body || !decorators.is_empty(),
                });
                collect_defs(body, out, blocked);
            }
            Stmt::GenDef { name, params, body, .. } => {
                out.push(Def { name, params, return_type: None, body, excluded: true });
                collect_defs(body, out, blocked);
            }
            Stmt::MetaFnDef { .. } | Stmt::MetaDecorated { .. } => {
                // メタ関数はコンパイル時に展開される。実行時の関数ではないので数えない
            }
            Stmt::ClassDef { body, .. } | Stmt::TraitDef { body, .. } => collect_defs(body, out, blocked),
            Stmt::Import { body, .. } | Stmt::FromImport { body, .. } => collect_defs(body, out, blocked),
            Stmt::If { .. }
            | Stmt::Match { .. }
            | Stmt::While { .. }
            | Stmt::For { .. }
            | Stmt::Block(_)
            | Stmt::Try { .. } => {
                // 制御構文の中の定義は、本体を借りたまま集められない（走査のクロージャの寿命）。名前だけ集めて
                // 「読むだけにしない名前」に入れる（同じ名前のほかの定義も巻き込むが、安全側）
                nested_def_names(std::slice::from_ref(s), blocked);
            }
            // 関数の定義を含まない文
            _ => {}
        }
    }
}

/// 文の並びの中（制御構文・入れ子の関数の本体を含む）にある関数の定義の名前を集める。
fn nested_def_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        if let Stmt::FnDef { name, .. } | Stmt::GenDef { name, .. } = s {
            out.insert(name.clone());
        }
        crate::stmt_walk::each_subpart(s, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            match part {
                P::Control(body) => nested_def_names(body, out),
                P::FnBody { body, .. } => nested_def_names(body, out),
                // 式の中のブロック・`gen` の本体・非同期の本体の中の定義は、その関数ごと条件 4 で外れる
                _ => {}
            }
        });
    }
}

/// 本体が `...` だけ（`.ars` のスタブの形）か。
fn is_stub_body(body: &[Stmt]) -> bool {
    matches!(body, [Stmt::Expr(Expr::Ident { name, .. })] if name == "Ellipsis" || name == "...")
}

/// 本体で宣言されるローカルの名前（`let` / `mut` / `const` / タプルの展開 / `for` の変数 / `except .. as e`）。
/// ⚠ 入れ子の定義・式の中のブロックには降りない（どちらも条件 4 で関数ごと外れるか、名前を足すだけで安全側）。
fn collect_locals(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        crate::decl_names::each_declared_name(s, &mut |name, _, _| {
            out.insert(name.to_string());
        });
        crate::stmt_walk::each_subpart(s, &mut |part| match part {
            crate::stmt_walk::StmtPart::Control(body) => collect_locals(body, out),
            crate::stmt_walk::StmtPart::ForTarget(t) | crate::stmt_walk::StmtPart::ExceptAlias(t) => {
                out.insert(t.to_string());
            }
            _ => {}
        });
    }
}

struct Ctx<'a> {
    /// 今の時点で読むだけと仮定している名前。
    readonly: &'a HashSet<&'a str>,
    /// この関数の仮引数とローカルの名前（同じ名前の関数を隠す）。
    locals: HashSet<String>,
    /// プログラムが `__iter__` / `__next__` を定義しているか（`for` が利用者のコードを走らせうる）。
    user_iter: bool,
    /// 属性・添字への代入の先として認める根の名前（4-6: この関数の `mut` 仮引数）。読むだけの判定では空。
    writable: HashSet<String>,
}

impl Ctx<'_> {
    fn declare_params(&mut self, params: &[Param]) {
        for p in params {
            self.locals.insert(p.name.clone());
        }
    }

    fn stmts_ok(&self, stmts: &[Stmt]) -> bool {
        stmts.iter().all(|s| self.stmt_ok(s))
    }

    /// 条件 4。⚠ ここに無い文は「読むだけでない」（分からないものは入れない）。
    fn stmt_ok(&self, s: &Stmt) -> bool {
        match s {
            Stmt::Expr(e) => self.expr_ok(e),
            Stmt::Let(_, _, e, _) | Stmt::Const(_, _, e, _) | Stmt::Mut(_, _, e, _) => self.expr_ok(e),
            Stmt::LetTuple { value, .. } => self.expr_ok(value),
            // 代入はローカルへだけ（大域・外側の変数を書き換えない）
            Stmt::Assign { name, value, .. } => self.locals.contains(name) && self.expr_ok(value),
            Stmt::CompoundAssign { name, value, .. } => self.locals.contains(name) && self.expr_ok(value),
            Stmt::Return(e, _) => e.as_ref().is_none_or(|e| self.expr_ok(e)),
            Stmt::BlockReturn(e, _) | Stmt::LoopYield(e, _) => self.expr_ok(e),
            // 属性・添字への代入は、自分の `mut` 仮引数を根にした経路へだけ（4-6。読むだけの判定では `writable` が空）
            Stmt::AttrAssign { target, value, .. } | Stmt::AttrCompoundAssign { target, value, .. } => {
                self.writable_target(target) && self.expr_ok(value)
            }
            Stmt::Break | Stmt::Continue | Stmt::Pass => true,
            Stmt::For { .. } if self.user_iter => false,
            Stmt::If { .. }
            | Stmt::Match { .. }
            | Stmt::While { .. }
            | Stmt::For { .. }
            | Stmt::Block(_)
            | Stmt::Try { .. }
            | Stmt::Raise { .. } => {
                let mut ok = true;
                crate::stmt_walk::each_subpart(s, &mut |part| {
                    use crate::stmt_walk::StmtPart as P;
                    ok = ok
                        && match part {
                            P::Expr(e) | P::MatchPattern(e) => self.expr_ok(e),
                            P::Control(body) => self.stmts_ok(body),
                            P::ForTarget(_) | P::ExceptAlias(_) | P::TargetName(_) => true,
                            // 入れ子の定義・非同期の本体は制御構文の直下には来ないが、来たら入れない
                            _ => false,
                        };
                });
                ok
            }
            // 属性・添字への代入、`static`、`freeze`、`yield`、定義、取り込み、非同期、イベント、デバッグ用の文
            _ => false,
        }
    }

    /// 代入の先が、`writable` の名前を根にした属性・添字の経路か（途中の添字の式も読むだけであること）。
    fn writable_target(&self, target: &Expr) -> bool {
        match target {
            Expr::Attr { object, .. } => self.writable_root(object),
            Expr::Subscript { object, index, .. } => self.writable_root(object) && self.expr_ok(index),
            _ => false,
        }
    }

    fn writable_root(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident { name, .. } => self.writable.contains(name),
            Expr::Attr { object, .. } => self.writable_root(object),
            Expr::Subscript { object, index, .. } => self.writable_root(object) && self.expr_ok(index),
            _ => false,
        }
    }

    fn args_ok(&self, args: &[CallArg]) -> bool {
        args.iter().all(|a| match a {
            CallArg::Positional(e) | CallArg::Keyword { value: e, .. } => self.expr_ok(e),
            // 展開（`*xs` / `**d`）・可変長は束縛の形が変わるので入れない
            _ => false,
        })
    }

    /// 条件 5。⚠ ここに無い式は「読むだけでない」。
    fn expr_ok(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::ImaginaryLit(_) | Expr::Str(_) | Expr::Bool(_) | Expr::None => true,
            Expr::Ident { .. } => true,
            Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => items.iter().all(|it| match it {
                SeqEntry::Item(e) | SeqEntry::Spread(e) => self.expr_ok(e),
            }),
            Expr::Dict(entries) => {
                let mut ok = true;
                for en in entries {
                    ok = ok && self.dict_entry_ok(en);
                }
                ok
            }
            Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => self.expr_ok(object),
            Expr::Subscript { object, index, .. } => self.expr_ok(object) && self.expr_ok(index),
            // `===`（同一性）は、受けた値が写しかどうかを見分けられる（写せば別の物・写さなければ同じ物）
            Expr::BinOp { op: crate::ast::BinOp::RefEq, .. } => false,
            Expr::BinOp { .. } | Expr::UnaryOp { .. } | Expr::Slice { .. } | Expr::Cast { .. } | Expr::IsType { .. }
            | Expr::MustBe { .. } => self.subparts_ok(e),
            Expr::ForExpr { .. } if self.user_iter => false,
            Expr::Block { .. } | Expr::IfExpr { .. } | Expr::ForExpr { .. } | Expr::WhileExpr { .. }
            | Expr::MatchExpr { .. } => self.subparts_ok(e),
            Expr::Call { func, args, .. } => {
                if !self.args_ok(args) {
                    return false;
                }
                match func.as_ref() {
                    // 名前の呼び出し: ローカルに隠されていない、読むだけの名前か、書き換えない組み込み関数
                    Expr::Ident { name, .. } => {
                        !self.locals.contains(name)
                            && (self.readonly.contains(name.as_str()) || READONLY_BUILTIN_FNS.contains(&name.as_str()))
                    }
                    // メソッドの呼び出し: 読むだけの名前か、書き換えない組み込み型のメソッド
                    Expr::Attr { object, attr, .. } => {
                        self.expr_ok(object)
                            && (self.readonly.contains(attr.as_str()) || is_readonly_builtin_method(attr))
                    }
                    // 関数の値・式の結果の呼び出し
                    _ => false,
                }
            }
            // テンプレートの具体化・メタ・デバッグ用の式・`Undefined` など
            _ => false,
        }
    }

    fn dict_entry_ok(&self, en: &DictEntry) -> bool {
        match en {
            DictEntry::Pair(k, v) => self.expr_ok(k) && self.expr_ok(v),
            DictEntry::Spread(e) => self.expr_ok(e),
        }
    }

    /// 直下の部分式・本体が全部読むだけか（演算子・式の形の制御構文）。
    fn subparts_ok(&self, e: &Expr) -> bool {
        let mut ok = true;
        crate::expr_walk::each_subpart(e, &mut |part| {
            use crate::expr_walk::SubPart as P;
            ok = ok
                && match part {
                    P::Plain(x) | P::Control(x) | P::MatchPattern(x) => self.expr_ok(x),
                    P::Body(body) => self.stmts_ok(body),
                    P::ForTarget(_) => true,
                };
        });
        ok
    }
}

/// 書き換えない組み込み型のメソッドの名前か: 組み込みの値のメソッド（[`crate::type_check::names::BUILTIN_VALUE_METHODS`]）で、
/// 受け手を書き換えるもの（`append`・`pop`・`sort`・`add`・`clear`…）と反復を作るもの（`__iter__`）を除く。
fn is_readonly_builtin_method(name: &str) -> bool {
    const MUTATING: &[&str] = &[
        "append", "pop", "add", "clear", "discard", "remove", "sort", "popitem", "setdefault", "update", "extend",
        "insert", "reverse", "write", "writelines", "close", "seek", "read", "readline", "readlines", "flush",
        "__iter__", "__next__", "send", "release", "emit",
    ];
    !MUTATING.contains(&name)
        && crate::type_check::names::BUILTIN_VALUE_METHODS.iter().any(|(_, ms)| ms.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::analyze;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn names(source: &str) -> Vec<String> {
        let tokens = Lexer::new(source, "").tokenize();
        let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
        analyze(&stmts).sorted_names().into_iter().map(str::to_string).collect()
    }

    const CLASS_V: &str = "class V:\n    mut x: int\n";

    /// 読むだけと判定するもの・しないものの表（copy_alloc_speed_plan.md 4-2）。
    #[test]
    fn verdicts() {
        let src = format!(
            "{CLASS_V}
    fn get(let self) -> int:
        return self.x
    fn add(let self, let o: V) -> int:
        return self.x + o.get()
    fn set(mut self, let v: int) -> None:
        self.x = v
    fn me(let self) -> V:
        return self
    fn same(let self, let o: V) -> bool:
        return self === o
    fn poke(let self, mut o: V) -> int:
        o.x = 1
        return self.x

fn total(let vs: list[V]) -> int:
    mut s = 0
    for v in vs:
        s += v.get()
    return s

fn grows(let vs: list[V]) -> int:
    mut ys = vs
    ys.append(V(1))
    return len(ys)

fn calls_value(let f: int, let v: V) -> int:
    return f(v)

mut counter = 0
fn bumps(let v: V) -> int:
    counter = counter + 1
    return v.x
"
        );
        assert_eq!(names(&src), ["add", "get", "total"]);
    }

    /// 自分の `mut` 仮引数だけを書き換える関数（4-6）。
    #[test]
    fn confined_writers() {
        let src = format!(
            "{CLASS_V}
    fn bump(mut self, let o: V) -> None:
        self.x = self.x + o.x
    fn leak(mut self, let o: V) -> V:
        self.x = o.x
        return o
    fn get(let self) -> int:
        return self.x

mut total = 0
fn tally(mut v: V) -> None:
    v.x = 0
    total = total + 1
"
        );
        let tokens = Lexer::new(&src, "").tokenize();
        let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
        let r = analyze(&stmts);
        // パーサが合成した `__init__`（`self.x = x` だけ）も入る
        assert_eq!(r.sorted_confined(), ["__init__", "bump"]);
        assert!(r.is_confined_writer("get") && r.is_confined_writer("bump") && !r.is_confined_writer("tally"));
    }

    /// 同じ名前の定義が 1 つでも外れれば、その名前は入らない。
    #[test]
    fn same_name_is_closed_over_every_definition() {
        let src = "class A:\n    mut k: int\n    fn read(let self) -> int:\n        return self.k\n\
                   class B:\n    mut k: int\n    fn read(mut self) -> int:\n        self.k = 0\n        return self.k\n\
                   fn use_a(let a: A) -> int:\n    return a.read()\n";
        assert!(names(src).is_empty());
    }

    /// 読むだけでない特殊メソッドがあれば、どの関数も入れない（演算子が何を呼ぶかは名前から分からない）。
    #[test]
    fn impure_special_method_clears_everything() {
        let ok = format!("{CLASS_V}    fn get(let self) -> int:\n        return self.x\n    fn __str__(let self) -> str:\n        return str(self.x)\n");
        assert_eq!(names(&ok), ["__str__", "get"]);
        let bad = format!("{CLASS_V}    fn get(let self) -> int:\n        return self.x\n    fn __str__(mut self) -> str:\n        self.x = 0\n        return str(self.x)\n");
        assert!(names(&bad).is_empty());
    }
}
