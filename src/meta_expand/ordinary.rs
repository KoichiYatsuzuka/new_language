// meta_expand/ordinary.rs — 展開後の通常コードに残ったメタ関数への言及を弾く（タスク 2-11）。
//
// ## 規則（設計書 §1.1・2026-09-25 確定）
//
//   * **メタ関数でない関数の中では、メタ関数を呼べない**（配置・純粋とも）。
//   * 配置メタ関数を呼べるのは、最上位とクラス本体に**文として**書いた呼び出しだけ
//     （そこは展開器が置いたコードで置き換える）。
//   * 参照は前方のみ（§1.7）。
//
// ## なぜ「展開後に 1 回歩く」のか
//
// 展開器が置き換えた呼び出し・装飾子・`const` の別名は、展開後の AST に**残らない**。
// ⇒ 展開後に残っているメタ関数への言及は**すべて誤り**。1 回歩けば漏れなく見つかる。
// ⚠ 以前（3-8）は実行時の `NameError` に理由を書き添えるだけだった。それだと
//   **その関数を呼ぶまで気づかない**（呼ばれない経路の誤りは黙って残る）。
//
// ## ⚠⚠ 保守的に倒している所
//
// **同じ名前の普通の束縛がプログラムのどこかにある**メタ関数は調べない。
// 局所変数や仮引数がメタ関数の名前を隠している場合を見分けるには、型検査と同じ
// スコープの追跡が要る。取り違えると**正しいプログラムを弾く**ので、疑わしい名前は
// 見送る。そのときは実行時の `NameError`（`explain_removed_metafn`）が網になる。

use std::collections::{HashMap, HashSet};

use crate::ast::{Expr, Stmt};
use crate::token::Span;

/// 通常コードから見える**メタ関数の名前**（タスク 2-11）。
///
/// ⚠ 置き換え済みの呼び出しは AST に残らないので、ここに載っている名前が展開後に
/// 見つかったら、それは置き換えられない位置に書かれていたということ。
pub(super) struct MetaNames<'a> {
    /// 素の名前 → 配置メタ関数か。
    pub(super) plain: &'a HashMap<String, bool>,
}

impl MetaNames<'_> {
    /// `e` がメタ関数を指していれば `(表示名, 配置メタ関数か)` を返す。
    fn lookup(&self, e: &Expr, shadowed: &HashSet<String>) -> Option<(String, bool)> {
        match e {
            Expr::Ident { name, .. } if !shadowed.contains(name) => {
                self.plain.get(name).map(|p| (name.clone(), *p))
            }
            _ => None,
        }
    }
}

/// 言及が置かれていた場所。文面を選ぶのに使う。
#[derive(Clone)]
enum Place {
    /// 最上位（またはクラス本体）の**式の位置**。
    Top,
    /// 最上位の制御構文の本体の中（関数の外）。
    Block,
    /// 普通の関数（メソッド・`gen` を含む）の本体の中。
    Function(String),
}

impl Place {
    /// 入れ子のブロックへ降りたときの場所。関数の中は関数のまま。
    fn nested(&self) -> Place {
        match self {
            Place::Top | Place::Block => Place::Block,
            Place::Function(f) => Place::Function(f.clone()),
        }
    }

    fn describe(&self) -> String {
        match self {
            Place::Top => "at the top level".to_string(),
            Place::Block => "inside a block".to_string(),
            Place::Function(f) => format!("inside function '{f}'"),
        }
    }
}

/// 展開後の文の列を歩いて、通常コードに残ったメタ関数への言及を探す（タスク 2-11）。
///
/// ⚠ 最初の 1 件で止める（他の展開エラーと同じ）。
/// ⚠ `import` 先のモジュール本体へは降りない。モジュールはそれぞれの展開で調べる（2-12）。
pub(super) fn check(stmts: &[Stmt], names: &MetaNames<'_>) -> Result<(), String> {
    // ⚠ メタ関数が 1 つも無くても歩く。入れ子のメタ関数の定義・通常コードの `^` は
    //   それだけで誤りなので。
    // ⚠ 普通の束縛（局所変数・仮引数・`for` の変数…）と同じ名前は見送る（モジュール doc）。
    let mut shadowed = HashSet::new();
    crate::interpreter::resolver::collect_bound_names(stmts, &mut shadowed);
    collect_params(stmts, &mut shadowed);
    let w = Walker { names, shadowed };
    w.stmts(stmts, &Place::Top)
}

/// すべての関数（入れ子を含む）の仮引数名を集める。
///
/// ⚠ `collect_bound_names` は**本体の中の宣言**しか拾わない（仮引数は拾わない）。
fn collect_params(stmts: &[Stmt], out: &mut HashSet<String>) {
    for st in stmts {
        if let Stmt::GenDef { params, .. } = st {
            out.extend(params.iter().map(|p| p.name.clone()));
        }
        crate::stmt_walk::each_subpart(st, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            match part {
                P::FnBody { params, body } => {
                    out.extend(params.iter().map(|p| p.name.clone()));
                    collect_params(body, out);
                }
                P::Control(b) | P::GenBody(b) | P::TypeBody(b) | P::AsyncBody(b) => {
                    collect_params(b, out)
                }
                P::Expr(e) | P::MatchPattern(e) => collect_params_in_expr(e, out),
                // 別モジュール・シグネチャだけの本体・束縛名は関係ない。
                P::ModuleBody(_)
                | P::ProtocolBody(_)
                | P::ForTarget(_)
                | P::ExceptAlias(_)
                | P::TargetName(_) => {}
            }
        });
    }
}

fn collect_params_in_expr(e: &Expr, out: &mut HashSet<String>) {
    crate::expr_walk::each_subpart(e, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) | P::MatchPattern(x) => collect_params_in_expr(x, out),
            P::Body(b) => collect_params(b, out),
            P::ForTarget(_) => {}
        }
    });
}

struct Walker<'a> {
    names: &'a MetaNames<'a>,
    shadowed: HashSet<String>,
}

impl Walker<'_> {
    fn stmts(&self, stmts: &[Stmt], place: &Place) -> Result<(), String> {
        for st in stmts {
            self.stmt(st, place)?;
        }
        Ok(())
    }

    fn stmt(&self, stmt: &Stmt, place: &Place) -> Result<(), String> {
        match stmt {
            // ⚠ 最上位のメタ関数の定義は展開器が消している。残っているのは入れ子の定義だけ。
            Stmt::MetaFnDef { name, .. } => {
                return Err(format!(
                    "MetaError: metafunction '{name}' is defined {} — metafunctions can only be \
                     defined at the top level of a file",
                    place.describe()
                ))
            }
            // ⚠ 最上位とクラス本体の `!装飾子` は展開器が消している。残っているのは
            //   関数の中・ブロックの中のものだけ。
            Stmt::MetaDecorated { decorators, spans, .. } => {
                let label = decorators
                    .first()
                    .map(super::decorator_label)
                    .unwrap_or_else(|| "?".to_string());
                return Err(format!(
                    "MetaError: decorator '!{label}' is written {} — `!` decorators are applied \
                     only at the top level or directly in a class body",
                    place.describe()
                ) + &at(spans.first()));
            }
            // ⚠⚠ **最上位（とクラス本体）に文として書いた配置メタ関数の呼び出し**が残っている
            //   ＝ 展開器から見えていなかった ＝ **宣言より前に呼んだ**（§1.7 の前方参照のみ）。
            //   「値として使った」とは別の誤りなので言い分ける。
            Stmt::Expr(Expr::Call { func, span, .. }) if matches!(place, Place::Top) => {
                if let Some((label, true)) = self.names.lookup(func, &self.shadowed) {
                    return Err(format!(
                        "MetaError: placing metafunction '{label}' is called before it is declared \
                         — expansion only sees metafunctions declared above the call (move the \
                         call below the declaration)"
                    ) + &at(Some(span)));
                }
            }
            _ => {}
        }
        let fn_name = match stmt {
            Stmt::FnDef { name, .. } | Stmt::GenDef { name, .. } => Some(name.clone()),
            _ => None,
        };
        let mut result = Ok(());
        crate::stmt_walk::each_subpart(stmt, &mut |part| {
            if result.is_err() {
                return;
            }
            use crate::stmt_walk::StmtPart as P;
            result = match part {
                P::Expr(e) | P::MatchPattern(e) => self.expr(e, place),
                P::Control(b) | P::AsyncBody(b) => self.stmts(b, &place.nested()),
                P::FnBody { body, .. } => {
                    self.stmts(body, &Place::Function(fn_name.clone().unwrap_or_default()))
                }
                P::GenBody(b) => {
                    self.stmts(b, &Place::Function(fn_name.clone().unwrap_or_default()))
                }
                // クラス本体の文はクラスと同じ場所（メソッドは `FnBody` で関数になる）。
                P::TypeBody(b) => self.stmts(b, place),
                // シグネチャ宣言だけ。
                P::ProtocolBody(_) => Ok(()),
                // 別モジュールはそれぞれの展開で調べる（2-12）。
                P::ModuleBody(_) => Ok(()),
                // 束縛名・代入先の名前は式ではない。
                P::ForTarget(_) | P::ExceptAlias(_) | P::TargetName(_) => Ok(()),
            };
        });
        result
    }

    fn expr(&self, e: &Expr, place: &Place) -> Result<(), String> {
        match e {
            // ⚠ `^` は展開時にしか評価できない。通常コードに残っていたら実行時に落ちる
            //   （3-9 の実行時の文面）ので、ここで先に止める。
            Expr::MetaInfo(target) => {
                let shown = match &**target {
                    Expr::Ident { name, .. } => format!("^{name}"),
                    _ => "^...".to_string(),
                };
                return Err(format!(
                    "MetaError: `{shown}` is written {} in ordinary code — `^` can only be \
                     evaluated while the program is being expanded: inside a metafunction, or as \
                     an argument to a metafunction call",
                    place.describe()
                ));
            }
            Expr::Call { func, span, .. } => {
                if let Some((label, placing)) = self.names.lookup(func, &self.shadowed) {
                    return Err(violation(&label, placing, place) + &at(Some(span)));
                }
            }
            _ => {
                if let Some((label, placing)) = self.names.lookup(e, &self.shadowed) {
                    return Err(violation(&label, placing, place));
                }
            }
        }
        let mut result = Ok(());
        crate::expr_walk::each_subpart(e, &mut |part| {
            if result.is_err() {
                return;
            }
            use crate::expr_walk::SubPart as P;
            result = match part {
                P::Plain(x) | P::Control(x) | P::MatchPattern(x) => self.expr(x, place),
                P::Body(b) => self.stmts(b, &place.nested()),
                P::ForTarget(_) => Ok(()),
            };
        });
        result
    }
}

/// 置き換えられない位置にあったメタ関数への言及の文面。
fn violation(label: &str, placing: bool, place: &Place) -> String {
    let what = if placing { "placing metafunction" } else { "pure metafunction" };
    match place {
        Place::Function(f) => format!(
            "MetaError: {what} '{label}' cannot be called inside function '{f}' — metafunctions \
             run while the program is being expanded, before any function runs (call a placing \
             metafunction as a statement at the top level or directly in a class body)"
        ),
        Place::Block => format!(
            "MetaError: {what} '{label}' cannot be called inside a block — the block runs at run \
             time, but a metafunction runs while the program is being expanded (call it directly \
             at the top level; to choose code at expansion time, branch on a `const` inside a \
             metafunction)"
        ),
        Place::Top if placing => format!(
            "MetaError: placing metafunction '{label}' cannot be used as a value — call it as a \
             statement, and the call is replaced by the code it places"
        ),
        Place::Top => format!(
            "MetaError: pure metafunction '{label}' cannot be called from ordinary code — it \
             returns `Code`, which exists only while the program is being expanded (call it from \
             another metafunction and `quote` what it returns)"
        ),
    }
}

/// 位置があれば出所の行として添える。
fn at(span: Option<&Span>) -> String {
    match span {
        Some(s) if s.line != 0 => format!("\n  at {s}"),
        _ => String::new(),
    }
}

/// プログラムのどこかに**メタ関数の構文**が書かれているか（タスク 2-11）。
///
/// ⚠ 最上位だけでなく、関数の中・ブロックの中・`import` 先のモジュール本体まで見る。
///   最上位だけを見ていると、関数の中に書いた `exprconst fn` や `^` が展開器の検査を
///   すり抜けて実行時まで届く。
/// ⚠ 読むだけの走査（複製しない）。メタ関数の無いプログラムでもこれだけは走る。
pub(super) fn mentions_meta(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_mentions_meta)
}

fn stmt_mentions_meta(st: &Stmt) -> bool {
    if matches!(st, Stmt::MetaFnDef { .. } | Stmt::MetaDecorated { .. }) {
        return true;
    }
    let mut found = false;
    crate::stmt_walk::each_subpart(st, &mut |part| {
        if found {
            return;
        }
        use crate::stmt_walk::StmtPart as P;
        found = match part {
            P::Expr(e) | P::MatchPattern(e) => expr_mentions_meta(e),
            P::Control(b)
            | P::GenBody(b)
            | P::TypeBody(b)
            | P::AsyncBody(b)
            | P::ProtocolBody(b)
            | P::ModuleBody(b) => mentions_meta(b),
            P::FnBody { body, .. } => mentions_meta(body),
            P::ForTarget(_) | P::ExceptAlias(_) | P::TargetName(_) => false,
        };
    });
    found
}

fn expr_mentions_meta(e: &Expr) -> bool {
    if matches!(e, Expr::MetaInfo(_)) {
        return true;
    }
    let mut found = false;
    crate::expr_walk::each_subpart(e, &mut |part| {
        if found {
            return;
        }
        use crate::expr_walk::SubPart as P;
        found = match part {
            P::Plain(x) | P::Control(x) | P::MatchPattern(x) => expr_mentions_meta(x),
            P::Body(b) => mentions_meta(b),
            P::ForTarget(_) => false,
        };
    });
    found
}
