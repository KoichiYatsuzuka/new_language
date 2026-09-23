// meta_expand/mod.rs — コンパイル時メタ関数の展開器（設計書 §1.7 / タスク 2-0）。
//
// ## 何をするか
//
// パース済みの文の列を**ファイル先頭から逐次に**歩き、
//
//   * `exprconst fn` / `exprconst !fn` の定義を**登録して AST から消す**
//   * 配置メタ関数の呼び出し文を、`quote` した `Code` の中身で**置き換える**
//   * `!装飾子` 付きの宣言を、装飾子が返した `Code` の中身で**置き換える**
//
// を行う。⚠ **繰り返し（不動点）はしない**（§1.7）。⇒ メタ関数・装飾子は
// **自分より前に宣言されたものだけ**を見られる。
//
// ## なぜ「置き換えてから続きを歩く」のか
//
// 置いたコードがさらにメタ関数を呼ぶことはある。これは**反復ではなく、同じ前向きの
// 走査を続けているだけ**なので §1.7 の「前方のみ参照」は壊れない。
// ⚠ ただし自分自身を置き続けるメタ関数は止まらないので、暫定の歩数上限を置いてある
// （ちゃんとした診断は 2-6）。
//
// ## メタ関数本体の実行
//
// **通常のインタプリタ（VM 経由）をそのまま使う。** メタ関数を `Stmt::FnDef` に写して
// 定義し、普通の関数として呼ぶ。
// ⚠ この方式が成立するのは 0-4 / 0-5 で評価コアを切り出してあるから。
// ⚠ `code:` は `Op::MakeCode` / `Op::Const`、`quote` は `Op::Return` に落ちる
//   （`src/vm/compiler/`）。⇒ 展開器のためだけの評価器は要らない。

use std::rc::Rc;

use crate::ast::{CodeLine, CodePiece, Expr, Stmt};
use crate::interpreter::{Interpreter, Value};
use crate::token::{Span, Spanned, Token};

/// 1 回の展開で歩ける文の数の上限（暫定）。
///
/// ⚠ 自分自身を置き続けるメタ関数は止まらないので、無いとハングする。
/// ⚠ **これは仮の柵**。歩数予算・再帰深さ・超過時の診断は 2-6 で作り直す。
const STEP_BUDGET: usize = 100_000;

/// 雛形に残っているスプライス（`<! !>`）の数。
///
/// ⚠ `Op::MakeCode` はこの数だけスタックから値を取る。**コンパイラが積む順と
/// ここが数える順は同じ**（行→要素の出現順）でなければならない。
pub fn splice_count(lines: &[CodeLine]) -> usize {
    lines
        .iter()
        .flat_map(|l| l.pieces.iter())
        .filter(|p| matches!(p, CodePiece::Splice(_)))
        .count()
}

/// 雛形のスプライス穴へ、評価済みの値を**出現順に**差し込む（設計書 §1.4）。
///
/// | 値 | 差し込まれるもの |
/// |---|---|
/// | `str` | 識別子 1 つ（変数名・関数名・型名） |
/// | `int` / `float` / `bool` | そのリテラル |
///
/// ⚠ **型と列はまだ扱えない**（設計書 §1.4 の「型」「列」）。型値の合成は 4-2、
/// 型文字列の検証は 4-3、引数シグネチャの列は 4-1。⇒ ここでは明示エラーにして、
/// 「黙って変なトークンが埋まる」ことを避ける。
pub fn fill_splices(lines: &[CodeLine], values: &[Value]) -> Result<Vec<CodeLine>, String> {
    let mut it = values.iter();
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let mut pieces = Vec::with_capacity(line.pieces.len());
        for piece in &line.pieces {
            match piece {
                CodePiece::Token(t) => pieces.push(CodePiece::Token(t.clone())),
                CodePiece::Splice(_) => {
                    let v = it.next().ok_or_else(|| {
                        "MetaError: internal — fewer spliced values than holes".to_string()
                    })?;
                    pieces.push(CodePiece::Token(splice_token(v)?));
                }
            }
        }
        out.push(CodeLine { pieces, indent: line.indent });
    }
    Ok(out)
}

/// スプライスされた 1 つの値をトークンへ写す。
///
/// ⚠ 位置は [`Span::unknown`]。展開由来のノードに元の位置を持たせるのは 2-3。
fn splice_token(v: &Value) -> Result<Spanned, String> {
    let token = match v {
        // ⚠ `str` は**識別子**になる（文字列リテラルではない）。設計書 §1.4 の一段目。
        //   名前を組み立てて差し込むのがスプライスの主用途なので、ここが既定。
        Value::Str(s) => Token::Ident(s.to_string()),
        Value::Int(n) => Token::Int(*n),
        Value::Float(f) => Token::Float(*f),
        Value::Bool(b) => {
            if *b {
                Token::True
            } else {
                Token::False
            }
        }
        other => {
            return Err(format!(
                "MetaError: `<! !>` cannot splice a '{}' yet \
                 (str / int / float / bool are supported; types and sequences come with tasks 4-1..4-3)",
                crate::interpreter::ops::typecheck::runtime_type_name(other)
            ))
        }
    };
    Ok(Spanned { token, span: Span::unknown() })
}

/// `Code`（未パースのトークン行）を文の列へ戻す（設計書 §1.2 / タスク 4-5 の①）。
///
/// ⚠⚠ **パースはここ 1 回だけ**。`code:` の中身を書いた時点ではパースしていないので、
/// 未完の断片（本体の無い `if` など）を組み立てられる。組み上がったものを最後に一度
/// パースする、というのが §1.2 の設計。
///
/// ⚠ 行に持っているのは**断片先頭からの相対インデント**なので、`Indent` / `Dedent` を
/// ここで作り直す。字句解析はやり直さない（トークンはもう持っている）。
pub fn code_to_stmts(lines: &[CodeLine]) -> Result<Vec<Stmt>, String> {
    code_to_stmts_in(lines, Context::TopLevel)
}

/// 置き先の文脈。**読み方が変わる**ので区別が要る（タスク 2-0）。
///
/// ⚠⚠ `mut x: int` は最上位では「初期値の無い変数宣言」でエラー、クラス本体では
/// フィールド宣言。⇒ 同じ `Code` でも置き先によってパースし分けなければならない。
/// ⚠ クラス本体に置ける種別の適合検査（`field`/`fn`/`gen` のみ等）は 2-5。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// 最上位・関数本体など、普通の文が書ける場所。
    TopLevel,
    /// クラス／トレイト本体。
    TypeBody,
}

/// 置き先の文脈を指定して `Code` を文の列へ戻す。
pub fn code_to_stmts_in(lines: &[CodeLine], ctx: Context) -> Result<Vec<Stmt>, String> {
    code_to_stmts_with(lines, ctx, None)
}

/// 置き先の文脈と **node-id カウンタ**を指定して `Code` を文の列へ戻す（タスク 2-1）。
///
/// ⚠⚠ **カウンタを渡さないと採番が 0 から始まり、本体の node-id と衝突する。**
/// node-id はプログラム全体で一意という前提で注釈が引かれる（設計書 §0.3）ので、
/// 衝突すると**別のノードの注釈を読む**。⇒ 展開器は必ず渡す。
/// ⚠ 同じ `Code` を 2 回置いても、そのたびにパースし直すので採番は自然に分かれる
/// （AST を複製して配り回さないので「クローン時の再採番」が要らない設計になっている）。
pub fn code_to_stmts_with(
    lines: &[CodeLine],
    ctx: Context,
    counter: Option<std::rc::Rc<std::cell::Cell<u32>>>,
) -> Result<Vec<Stmt>, String> {
    let mut tokens: Vec<Spanned> = Vec::new();
    let unknown = || Span::unknown();
    let mut depth: i32 = 0;
    for line in lines {
        while depth < line.indent {
            tokens.push(Spanned { token: Token::Indent, span: unknown() });
            depth += 1;
        }
        while depth > line.indent {
            tokens.push(Spanned { token: Token::Dedent, span: unknown() });
            depth -= 1;
        }
        for piece in &line.pieces {
            match piece {
                CodePiece::Token(t) => tokens.push(t.clone()),
                // `Op::MakeCode` を通っていれば穴は残らない。残っているのは
                // 「スプライス付きの `Code` を評価せずに配置しようとした」ときだけ。
                CodePiece::Splice(_) => {
                    return Err(
                        "MetaError: this `Code` still has an unevaluated `<! !>` splice".to_string()
                    )
                }
            }
        }
        tokens.push(Spanned { token: Token::Newline, span: unknown() });
    }
    while depth > 0 {
        tokens.push(Spanned { token: Token::Dedent, span: unknown() });
        depth -= 1;
    }
    tokens.push(Spanned { token: Token::Eof, span: unknown() });

    let mut parser = crate::parser::Parser::new(tokens, None);
    if let Some(c) = counter {
        parser.set_node_counter(c);
    }
    let parsed = match ctx {
        Context::TopLevel => parser.parse_program(),
        Context::TypeBody => parser.parse_class_body_fragment(),
    };
    parsed.map_err(|e| format!("MetaError: the placed `Code` does not parse: {e}"))
}

// ── 展開器本体 ───────────────────────────────────────────────────────────────

/// 展開器の状態。メタ関数の登録簿と、それを実行するインタプリタを持つ。
struct Expander {
    /// メタ関数名 → 配置メタ関数（`exprconst !fn`）かどうか。
    ///
    /// ⚠ 実体（呼べる関数値）は `interp` のグローバルスコープに入っている。
    /// ここが持つのは「メタ関数である」ことと種別だけ。
    metafns: std::collections::HashMap<String, bool>,
    /// メタ関数本体を走らせるインタプリタ。
    ///
    /// ⚠ **通常実行のインタプリタとは別物**。展開時に作った変数が実行時へ漏れない。
    interp: Interpreter,
    /// 歩数（`STEP_BUDGET` の判定用）。
    steps: usize,
    /// 置いたコードを採番する node-id カウンタ（設計書 §0.3 / タスク 2-1）。
    ///
    /// ⚠⚠ **本体をパースしたパーサと同じものを渡す。** 別のカウンタだと node-id が
    /// 衝突し、型検査の注釈が**別のノードのもの**になる（per-module 採番で実際に
    /// FFI 境界検査の誤検知が再現した — `Parser::node_counter` の doc）。
    node_counter: std::rc::Rc<std::cell::Cell<u32>>,
}

impl Expander {
    fn new(node_counter: std::rc::Rc<std::cell::Cell<u32>>) -> Self {
        Self {
            metafns: std::collections::HashMap::new(),
            interp: Interpreter::new(),
            steps: 0,
            node_counter,
        }
    }

    /// 置き先の文脈に合わせて `Code` を文へ戻す（採番は本体と共有）。
    fn place(&self, code: &[CodeLine], ctx: Context) -> Result<Vec<Stmt>, String> {
        code_to_stmts_with(code, ctx, Some(std::rc::Rc::clone(&self.node_counter)))
    }

    /// メタ関数を「普通の関数」としてインタプリタに登録する。
    ///
    /// ⚠ `Stmt::MetaFnDef` のまま `exec` へ渡すと「展開されていない」エラーになるので、
    /// ここで [`Stmt::FnDef`] へ写す。本体はそのまま（`code:` / `quote` は VM が扱う）。
    fn define_metafn(&mut self, def: &Stmt) -> Result<(), String> {
        let Stmt::MetaFnDef { name, params, return_type, body, is_placing } = def else {
            return Err("MetaError: internal — define_metafn on a non-metafunction".to_string());
        };
        self.metafns.insert(name.clone(), *is_placing);
        let as_fn = Stmt::FnDef {
            name: name.clone(),
            template_params: Vec::new(),
            params: params.clone(),
            body: body.clone(),
            decorators: Vec::new(),
            return_type: return_type.clone(),
            is_static: false,
            is_class_method: false,
            is_abstract: false,
            access: crate::ast::Accessibility::Public,
        };
        self.interp.exec(&as_fn).map(|_| ())
    }

    /// メタ関数を呼んで `Code` を得る。
    fn call_metafn(&mut self, name: &str, args: Vec<Value>) -> Result<Rc<Vec<CodeLine>>, String> {
        let callee = self
            .interp
            .vm_load_name(name)
            .ok_or_else(|| format!("MetaError: metafunction '{name}' is not defined"))?;
        let evaled: Vec<(Option<String>, Value, bool)> =
            args.into_iter().map(|v| (None, v, false)).collect();
        let out = self
            .interp
            .call_value_evaled(callee, evaled, name, None, 0)
            .map_err(|e| format!("MetaError: while expanding '{name}': {e}"))?;
        match out {
            Value::Code(lines) => Ok(lines),
            // ⚠ `quote` に到達せず抜けた形（設計書 §1.1）。診断の本番は 3-5。
            Value::None => Err(format!(
                "MetaError: metafunction '{name}' returned without placing any `Code` \
                 (use `quote` — an empty `code:` block places nothing)"
            )),
            other => Err(format!(
                "MetaError: metafunction '{name}' must produce `Code`, got '{}'",
                self.interp.type_name(&other)
            )),
        }
    }
}

/// 文の列を展開する（設計書 §1.7）。入力を壊さず、新しい列を返す。
///
/// ⚠⚠ **`resolve_and_annotate` より前に走らせること。** 展開で生えた宣言を
/// リゾルバ・型検査・VM が見られるようにするため。配線は 2-2。
pub fn expand_program(
    stmts: Vec<Stmt>,
    node_counter: std::rc::Rc<std::cell::Cell<u32>>,
) -> Result<Vec<Stmt>, String> {
    let mut ex = Expander::new(node_counter);
    expand_stmts(&mut ex, stmts, Context::TopLevel)
}

/// 1 つの文の列を前から順に展開する。
fn expand_stmts(
    ex: &mut Expander,
    stmts: Vec<Stmt>,
    ctx: Context,
) -> Result<Vec<Stmt>, String> {
    // ⚠ 置換で文が増えるので、**残りを持ち回る**形にする（`for` では書けない）。
    let mut pending: std::collections::VecDeque<Stmt> = stmts.into();
    let mut out: Vec<Stmt> = Vec::new();

    while let Some(stmt) = pending.pop_front() {
        ex.steps += 1;
        if ex.steps > STEP_BUDGET {
            return Err(format!(
                "MetaError: expansion did not finish within {STEP_BUDGET} steps \
                 (a metafunction is probably placing a call to itself)"
            ));
        }

        match stmt {
            // 定義は登録して AST から消す。
            Stmt::MetaFnDef { .. } => ex.define_metafn(&stmt)?,

            // 配置メタ関数の呼び出し文 → `quote` した中身で置き換える。
            Stmt::Expr(Expr::Call { ref func, ref args, .. })
                if placing_call_name(ex, func).is_some() =>
            {
                let name = placing_call_name(ex, func).expect("checked by the guard");
                let vals = eval_args(ex, args)?;
                let code = ex.call_metafn(&name, vals)?;
                let placed = ex.place(&code, ctx)?;
                // ⚠ **置いたものを先に歩く**。置いたコードがさらにメタ関数を呼ぶことはある。
                for s in placed.into_iter().rev() {
                    pending.push_front(s);
                }
            }

            // `!装飾子` 付きの宣言 → 装飾子が返した中身で置き換える。
            Stmt::MetaDecorated { decorators, target } => {
                let placed = apply_decorators(ex, decorators, *target, ctx)?;
                for s in placed.into_iter().rev() {
                    pending.push_front(s);
                }
            }

            // クラス／トレイト本体もその場で展開する（メンバーの装飾子がここで消える）。
            Stmt::ClassDef {
                name,
                template_params,
                bases,
                base_args,
                body,
                decorators,
            } => {
                let body = expand_stmts(ex, body, Context::TypeBody)?;
                out.push(Stmt::ClassDef {
                    name,
                    template_params,
                    bases,
                    base_args,
                    body,
                    decorators,
                });
            }
            Stmt::TraitDef { name, template_params, body } => {
                let body = expand_stmts(ex, body, Context::TypeBody)?;
                out.push(Stmt::TraitDef { name, template_params, body });
            }

            // それ以外はそのまま。
            // ⚠ **関数本体の中は今は歩いていない**（§1.7 はファイル先頭からの逐次展開で、
            //   本体の中の展開は別途決める必要がある）。⇒ 未対応として記録済み。
            other => out.push(other),
        }
    }
    Ok(out)
}

/// 呼び出し式が「登録済みの**配置**メタ関数」ならその名前を返す。
fn placing_call_name(ex: &Expander, func: &Expr) -> Option<String> {
    let Expr::Ident { name, .. } = func else { return None };
    match ex.metafns.get(name) {
        Some(true) => Some(name.clone()),
        _ => None,
    }
}

/// 実引数を展開時に評価する。
///
/// ⚠ 展開時に確定しないもの（実行時の変数など）はここで失敗する。
/// 「何が確定するか」の診断は 3-0（§1.8）。
fn eval_args(ex: &mut Expander, args: &[crate::ast::CallArg]) -> Result<Vec<Value>, String> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        match a {
            crate::ast::CallArg::Positional(e) => out.push(ex.interp.eval(e)?),
            _ => {
                return Err(
                    "MetaError: metafunction calls take positional arguments only (for now)"
                        .to_string(),
                )
            }
        }
    }
    Ok(out)
}

/// `!装飾子` を適用して、置き換える文の列を返す（設計書 §1.6）。
///
/// ⚠ **内側から**適用する（§1.6）。宣言に一番近い装飾子＝リストの**末尾**が先。
/// ⚠ 対象は**第一引数**として渡る。渡す形は `ast_value` の AST 値（4-0 で `meta_*` に寄せる）。
fn apply_decorators(
    ex: &mut Expander,
    decorators: Vec<Expr>,
    target: Stmt,
    ctx: Context,
) -> Result<Vec<Stmt>, String> {
    let mut current = vec![target];
    for deco in decorators.into_iter().rev() {
        let Expr::Ident { ref name, .. } = deco else {
            return Err(
                "MetaError: only a plain decorator name is supported so far \
                 (arguments and closures come later)"
                    .to_string(),
            )
        };
        if !ex.metafns.contains_key(name) {
            return Err(format!(
                "MetaError: decorator '!{name}' is not a metafunction declared before this point"
            ));
        }
        if current.len() != 1 {
            return Err(format!(
                "MetaError: decorator '!{name}' cannot be applied — the inner decorator produced \
                 {} statements, and a decorator takes exactly one declaration",
                current.len()
            ));
        }
        let arg = crate::interpreter::ast_value::stmt_to_value(&current[0]);
        let code = ex.call_metafn(name, vec![arg])?;
        current = ex.place(&code, ctx)?;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ソースを展開して、残った文の種別名を並べたものを返す。
    fn expand_kinds(src: &str) -> Result<Vec<&'static str>, String> {
        let out = expand(src)?;
        Ok(out.iter().map(crate::interpreter::tw_stats::stmt_kind_of).collect())
    }

    /// パースして、文の列と**そのパーサが使った node-id カウンタ**を返す。
    ///
    /// ⚠ カウンタを展開器へ渡すのが要点（設計書 §0.3 / タスク 2-1）。
    fn parse(src: &str) -> Result<(Vec<Stmt>, std::rc::Rc<std::cell::Cell<u32>>), String> {
        let tokens = crate::lexer::Lexer::new(src, "").tokenize();
        let mut parser = crate::parser::Parser::new(tokens, None);
        let stmts = parser.parse_program()?;
        Ok((stmts, parser.node_counter()))
    }

    fn expand(src: &str) -> Result<Vec<Stmt>, String> {
        let (stmts, counter) = parse(src)?;
        expand_program(stmts, counter)
    }

    /// メタ関数の定義は**登録されて AST から消える**（設計書 §1.7）。
    /// ⚠ 消えないと、展開後の AST に実行できない文が残る。
    #[test]
    fn metafn_definitions_are_removed_from_the_ast() {
        let kinds = expand_kinds(
            "exprconst !fn place_it() -> None:\n    quote code:\n\nprint(1)\n",
        )
        .expect("expand");
        assert_eq!(kinds, vec!["Expr"], "`print(1)` だけが残る");
    }

    /// 配置メタ関数の呼び出しが、`quote` した `Code` の中身で置き換わる。
    #[test]
    fn a_placing_metafn_call_is_replaced_by_its_code() {
        let out = expand(
            "exprconst !fn place_it() -> None:\n    quote code:\n        let placed = 1\n\nplace_it()\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(
            matches!(&out[0], Stmt::Let(name, _, _) if name == "placed"),
            "置かれたのは `let placed = 1`: {:?}",
            crate::interpreter::tw_stats::stmt_kind_of(&out[0])
        );
    }

    /// 空の `code:` を `quote` すると**何も置かれない**（設計書 §1.2 の B-2）。
    #[test]
    fn quoting_an_empty_code_block_places_nothing() {
        let kinds = expand_kinds(
            "exprconst !fn place_nothing() -> None:\n    quote code:\n\nplace_nothing()\nprint(1)\n",
        )
        .expect("expand");
        assert_eq!(kinds, vec!["Expr"], "`print(1)` だけが残る");
    }

    /// スプライス `<! !>` が展開時の値で埋まる（設計書 §1.4）。
    /// ⚠ `str` は**識別子**になる（文字列リテラルではない）。
    #[test]
    fn a_splice_fills_in_an_identifier() {
        let out = expand(
            "exprconst !fn bind(n: str) -> None:\n    quote code:\n        let <! n !> = 1\n\nbind(\"chosen\")\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(
            matches!(&out[0], Stmt::Let(name, _, _) if name == "chosen"),
            "スプライスした名前で束縛される"
        );
    }

    /// 置いたコードがさらにメタ関数を呼ぶ形も、同じ前向きの走査で処理される。
    /// ⚠ これは**反復ではない**（§1.7 の「前方のみ参照」は保たれる）。
    #[test]
    fn code_placed_by_a_metafn_is_expanded_too() {
        let out = expand(
            "exprconst !fn inner() -> None:\n    quote code:\n        let deep = 1\n\nexprconst !fn outer() -> None:\n    quote code:\n        inner()\n\nouter()\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Stmt::Let(name, _, _) if name == "deep"));
    }

    /// ⚠ `quote` に到達せず抜けたら**エラー**（設計書 §1.1）。
    /// 黙って無視すると「メタ関数を書いたのに何も起きない」になる。
    #[test]
    fn a_placing_metafn_that_never_quotes_is_an_error() {
        let err = expand("exprconst !fn nothing() -> None:\n    pass\n\nnothing()\n")
            .expect_err("エラーになること");
        assert!(err.contains("without placing any `Code`"), "実際のエラー: {err}");
    }

    /// ⚠ 自分自身を置き続けるメタ関数は止まらない。⇒ 歩数上限で切る（本番の診断は 2-6）。
    #[test]
    fn a_self_placing_metafn_is_stopped_by_the_step_budget() {
        let err = expand(
            "exprconst !fn loop_forever() -> None:\n    quote code:\n        loop_forever()\n\nloop_forever()\n",
        )
        .expect_err("止まること");
        assert!(err.contains("did not finish within"), "実際のエラー: {err}");
    }

    /// `!装飾子` が対象の宣言を置き換える（設計書 §1.6）。
    #[test]
    fn a_decorator_replaces_its_target() {
        let out = expand(
            "exprconst !fn swap(target) -> None:\n    quote code:\n        let swapped = 1\n\n!swap\nlet original = 0\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Stmt::Let(name, _, _) if name == "swapped"));
    }

    /// ⚠ 自分より**前**に宣言されたメタ関数しか見えない（§1.7 の逐次展開）。
    /// 後ろにあるものを呼んでも展開されず、そのまま残る（実行時に NameError になる）。
    #[test]
    fn a_metafn_declared_later_is_not_visible() {
        let kinds = expand_kinds(
            "later()\nexprconst !fn later() -> None:\n    quote code:\n        let x = 1\n",
        )
        .expect("expand");
        assert_eq!(kinds, vec!["Expr"], "呼び出し文が展開されずに残る");
    }

    /// ⚠⚠ 置いたコードの node-id は**本体と衝突してはならない**（設計書 §0.3 / タスク 2-1）。
    /// 衝突すると型検査の注釈が**別のノードのもの**になる（per-module 採番で FFI 境界検査の
    /// 誤検知が実際に再現した）。⇒ 展開器は本体と同じカウンタから採番する。
    #[test]
    fn placed_nodes_do_not_reuse_node_ids() {
        let src = concat!(
            "exprconst !fn place_it() -> None:\n",
            "    quote code:\n",
            "        let placed = other\n",
            "\n",
            "let other = 1\n",
            "let a = other\n",
            "let b = other\n",
            "let c = other\n",
            "place_it()\n",
        );
        let (stmts, counter) = parse(src).expect("parse");
        // 本体をパースし終えた時点の採番位置。置いたコードはこれより**後ろ**から採番される。
        let before = counter.get();
        assert!(before > 0, "本体が node-id を採番していること");
        let out = expand_program(stmts, std::rc::Rc::clone(&counter)).expect("expand");

        let mut ids: Vec<u32> = Vec::new();
        collect_node_ids(&out, &mut ids);
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "node-id が重複している: {ids:?}");
        assert!(!ids.contains(&0), "未採番（0）が混ざっている: {ids:?}");
        // ⚠⚠ **これが本命の検査**。カウンタを共有していなければ置いたコードは 1 から
        //   採番され、`before` を超える id は 1 つも出ない（＝ここで落ちる）。
        assert!(
            ids.iter().any(|id| *id > before),
            "置いたコードが本体と同じカウンタから採番されていない (before={before}, ids={ids:?})"
        );
    }

    /// テスト用: 文の列に現れる `Expr::Ident` の node-id を集める。
    fn collect_node_ids(stmts: &[Stmt], out: &mut Vec<u32>) {
        for st in stmts {
            crate::stmt_walk::each_subpart(st, &mut |part| {
                if let crate::stmt_walk::StmtPart::Expr(e) = part {
                    collect_expr_ids(e, out);
                }
            });

        }
    }

    fn collect_expr_ids(e: &Expr, out: &mut Vec<u32>) {
        if let Expr::Ident { node_id, .. } = e {
            out.push(*node_id);
        }
        crate::expr_walk::each_subpart(e, &mut |sub| {
            if let crate::expr_walk::SubPart::Plain(x) | crate::expr_walk::SubPart::Control(x) = sub
            {
                collect_expr_ids(x, out);
            }
        });
    }

    /// クラス本体のメンバー装飾子もその場で展開される。
    /// ⚠ ここを歩かないと、クラスのメンバーに付けた装飾子が**黙って無視される**。
    #[test]
    fn decorators_inside_a_class_body_are_expanded() {
        let out = expand(
            "exprconst !fn tag(target) -> None:\n    quote code:\n        mut tagged: int\n\nclass C:\n    !tag\n    mut original: int\n",
        )
        .expect("expand");
        let Stmt::ClassDef { body, .. } = &out[0] else { panic!("ClassDef を期待") };
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0], Stmt::Field { name, .. } if name == "tagged"));
    }
}
